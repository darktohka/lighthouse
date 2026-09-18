//! Bounded, self-expiring cache of decompressed layer filesystem indices.
//!
//! A tar stream has no central directory, so building a layer index means
//! decompressing and scanning the whole archive. The layer browser therefore
//! keeps the merged path map here once it has been built, and serves later
//! `tree` requests from memory. The cache is bounded three ways:
//!
//! * **Weight** — every index is charged an estimated heap footprint and the
//!   running total is capped by [`LayerCacheConfig::max_bytes`]. An index that
//!   alone exceeds the budget is never stored.
//! * **LRU** — while over budget the least-recently-used indices are evicted
//!   first; reading an index refreshes its position.
//! * **TTL** — an index older than [`LayerCacheConfig::ttl`] is treated as a
//!   miss and dropped, so a cached view can never outlive its TTL by more than
//!   the gap between requests. Expired indices are reclaimed by a sweep that
//!   runs only once the earliest entry is actually due — the deadline is
//!   tracked in `next_expires`, never recomputed on a fixed cadence.
//!
//! Concurrent misses for the same digest are collapsed onto one build, so a
//! cold layer is decompressed once even under a burst of requests.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bytes::Bytes;

use crate::error::{ApiError, ApiResult};
use crate::oci::digest::Digest;

/// Estimated fixed cost of one cached entry: the map node plus the inline
/// fields of [`CachedEntry`]. Path and symlink bytes are charged on top.
const ENTRY_OVERHEAD_BYTES: usize = 64;

/// Estimated fixed cost of one `Arc<str>` path key: the two atomic reference
/// counts plus the length field. The path bytes are charged separately.
const ARC_STR_HEADER_BYTES: usize = 16;

/// Estimated per-entry overhead of a persistent hash-map slot and its control
/// bytes.
const HASH_SLOT_BYTES: usize = 16;

/// Estimated fixed cost of one cached listing key beyond its own string bytes.
const LISTING_KEY_OVERHEAD_BYTES: usize = 64;

/// Maximum number of symlink hops followed before a link is treated as
/// unresolvable.
const MAX_SYMLINK_DEPTH: usize = 16;

/// The kind of filesystem object an index entry describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
}

impl EntryKind {
    /// The wire value used by the layer browser API.
    pub fn as_str(self) -> &'static str {
        match self {
            EntryKind::Dir => "dir",
            EntryKind::File => "file",
            EntryKind::Symlink => "symlink",
        }
    }
}

/// One filesystem object in a cached layer index.
#[derive(Debug, Clone)]
pub struct CachedEntry {
    pub kind: EntryKind,
    pub size: i64,
    pub mode: u32,
    pub link_target: Option<Box<str>>,
}

impl CachedEntry {
    /// A synthesized directory prefix that never appeared as a tar header.
    pub fn directory() -> Self {
        Self {
            kind: EntryKind::Dir,
            size: 0,
            mode: 0o755,
            link_target: None,
        }
    }

    fn weight(&self) -> usize {
        ENTRY_OVERHEAD_BYTES + self.link_target.as_deref().map_or(0, str::len)
    }
}

/// The merged, whiteout-resolved path map of one layer. Immutable once built
/// and shared between requests, so it only needs to be cheap to read.
#[derive(Debug, Default)]
pub struct LayerIndex {
    entries: HashMap<String, CachedEntry>,
    /// Subtree roots this layer removes: a `.wh.<name>` target path and the
    /// directory made opaque by `.wh..wh..opq`. Removal covers the whole
    /// subtree, so no entry under a root survives in a later overlay even when
    /// its intermediate directories never appeared as explicit tar headers.
    deletes: HashSet<String>,
    weight: usize,
}

impl LayerIndex {
    /// Wraps an already-merged path map with no deletions, computing the weight
    /// once.
    pub fn from_entries(entries: HashMap<String, CachedEntry>) -> Self {
        Self::with_deletes(entries, HashSet::new())
    }

    /// Wraps an already-merged path map together with the subtree roots this
    /// layer removes, computing the weight once.
    pub fn with_deletes(entries: HashMap<String, CachedEntry>, deletes: HashSet<String>) -> Self {
        let entry_weight = entries
            .iter()
            .map(|(path, entry)| path.len() + entry.weight())
            .sum::<usize>();
        let delete_weight = deletes
            .iter()
            .map(|path| path.len() + ENTRY_OVERHEAD_BYTES)
            .sum::<usize>();
        Self {
            entries,
            deletes,
            weight: entry_weight + delete_weight,
        }
    }

    /// The merged path map, keyed by normalized path.
    pub fn entries(&self) -> &HashMap<String, CachedEntry> {
        &self.entries
    }

    /// The subtree roots this layer removes.
    pub fn deletes(&self) -> &HashSet<String> {
        &self.deletes
    }

    /// Estimated heap footprint of this index in bytes.
    pub fn weight(&self) -> usize {
        self.weight
    }

    /// Number of indexed paths.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resolves a symlink's raw `target` (relative to the symlink's own
    /// directory, or absolute inside the layer) to an existing entry, following
    /// chains. Returns the normalized path and the final entry's kind, or
    /// `None` when the link is dangling, cyclic, or escapes the layer.
    pub fn resolve_link(&self, link_path: &str, target: &str) -> Option<(String, EntryKind)> {
        resolve_link_in(&self.entries, link_path, target)
    }
}

/// Resolves a symlink against a path map, following chains to a concrete entry.
fn resolve_link_in(
    entries: &HashMap<String, CachedEntry>,
    link_path: &str,
    target: &str,
) -> Option<(String, EntryKind)> {
    let mut current = normalize_link_target(parent_directory(link_path), target)?;
    for _ in 0..MAX_SYMLINK_DEPTH {
        match entry_kind_in(entries, &current)? {
            EntryKind::Symlink => {
                let next = entries.get(&current)?.link_target.as_deref()?;
                current = normalize_link_target(parent_directory(&current), next)?;
            }
            kind => return Some((current, kind)),
        }
    }
    None
}

/// The kind at `path`, recognizing directories that exist only as a prefix of
/// deeper entries.
fn entry_kind_in(entries: &HashMap<String, CachedEntry>, path: &str) -> Option<EntryKind> {
    if let Some(entry) = entries.get(path) {
        return Some(entry.kind);
    }
    if path.is_empty() {
        return Some(EntryKind::Dir);
    }
    let prefix = format!("{path}/");
    entries
        .keys()
        .any(|candidate| candidate.starts_with(&prefix))
        .then_some(EntryKind::Dir)
}

/// True when `path` itself or any of its ancestors is a delete root.
fn removed_by(path: &str, deletes: &HashSet<String>) -> bool {
    if deletes.is_empty() {
        return false;
    }
    if deletes.contains("") {
        return true;
    }
    let mut candidate = path;
    loop {
        if deletes.contains(candidate) {
            return true;
        }
        match candidate.rsplit_once('/') {
            Some((parent, _)) => candidate = parent,
            None => return false,
        }
    }
}

/// Collects every directory path implied by a path map: explicit directory
/// entries and the ancestors of every entry, so a directory that exists only as
/// a prefix is represented too.
fn ancestor_dirs<'a, I>(entries: I, dirs: &mut HashSet<String>)
where
    I: IntoIterator<Item = (&'a str, bool)>,
{
    for (path, is_dir) in entries {
        if is_dir {
            dirs.insert(path.to_string());
        }
        add_ancestors(path, dirs);
    }
}

/// Inserts every ancestor directory of `path` into `target`.
fn add_ancestors(path: &str, target: &mut HashSet<String>) {
    let mut cursor = path;
    while let Some((parent, _)) = cursor.rsplit_once('/') {
        target.insert(parent.to_string());
        cursor = parent;
    }
}

/// Two entries are unchanged only when every observable attribute matches; a
/// re-added identical file therefore carries no colour.
fn same_entry(a: &CachedEntry, b: &CachedEntry) -> bool {
    a.kind == b.kind && a.size == b.size && a.mode == b.mode && a.link_target == b.link_target
}

/// How one path changed in the upper overlay relative to everything below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    New,
    Modified,
    Removed,
}

impl Change {
    /// The wire value used by the layer browser API.
    pub fn as_str(self) -> &'static str {
        match self {
            Change::New => "new",
            Change::Modified => "modified",
            Change::Removed => "removed",
        }
    }
}

/// A shared, immutable path key. Cloning the `Arc` shares the underlying
/// string, so a path repeated across cumulative overlays is stored once.
pub type PathRef = std::sync::Arc<str>;

/// One path in a cumulative overlay: the merged entry plus the index of the
/// manifest layer that last supplied it.
#[derive(Debug, Clone)]
pub struct Node {
    pub entry: CachedEntry,
    pub source: Option<u32>,
}

/// The cumulative overlay of a manifest's layers up to one position.
///
/// The merged path map is a single persistent, structurally-shared
/// [`im::HashMap`]: composing the next position clones the map in O(1) and
/// touches only the paths the new layer adds or removes. Each node records the
/// manifest-layer index that supplied it instead of a cloned [`Digest`], so a
/// digest is never duplicated per path.
#[derive(Debug, Default)]
pub struct ComposedLayer {
    nodes: im::HashMap<PathRef, Node>,
    layers: std::sync::Arc<Vec<Digest>>,
    weight: usize,
}

impl ComposedLayer {
    /// An empty overlay, the identity for [`ComposedLayer::compose`].
    pub fn empty() -> Self {
        Self::default()
    }

    /// Overlays `upper` onto `base` at manifest-layer `position`, recording
    /// that position as the source of every path `upper` supplies.
    ///
    /// `layers` is the manifest's ordered layer-digest table and `position` is
    /// an index into it. The map shares structure with `base`, so composing an
    /// N-layer manifest never re-hashes the layers beneath it.
    pub fn compose(
        base: &ComposedLayer,
        upper: &LayerIndex,
        position: u32,
        layers: std::sync::Arc<Vec<Digest>>,
    ) -> Self {
        let mut nodes = base.nodes.clone();
        if !upper.deletes().is_empty() {
            let deletes = upper.deletes();
            nodes.retain(|path, _| !removed_by(path, deletes));
        }
        for (path, entry) in upper.entries() {
            let key = match base.nodes.get_key_value(path.as_str()) {
                Some((existing, _)) => std::sync::Arc::clone(existing),
                None => PathRef::from(path.as_str()),
            };
            nodes.insert(
                key,
                Node {
                    entry: entry.clone(),
                    source: Some(position),
                },
            );
        }
        let weight = node_map_weight(&nodes);
        Self {
            nodes,
            layers,
            weight,
        }
    }

    /// The digest of the manifest layer that last supplied `path`, when known.
    pub fn source(&self, path: &str) -> Option<&Digest> {
        self.source_at(self.nodes.get(path)?.source?)
    }

    /// The digest at manifest-layer `index`, when the table holds it.
    pub fn source_at(&self, index: u32) -> Option<&Digest> {
        self.layers.get(index as usize)
    }

    /// The merged nodes, keyed by normalized path.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Node)> {
        self.nodes.iter().map(|(path, node)| (&**path, node))
    }

    /// The merged entries without their source index.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &CachedEntry)> {
        self.nodes.iter().map(|(path, node)| (&**path, &node.entry))
    }

    /// The node at `path`, when present.
    pub fn get(&self, path: &str) -> Option<&Node> {
        self.nodes.get(path)
    }

    /// Estimated heap footprint of this overlay in bytes.
    ///
    /// Structural sharing is not modelled: a node shared with a lower position
    /// is counted again here. The over-count never under-estimates, at the cost
    /// of evicting a shared position early.
    pub fn weight(&self) -> usize {
        self.weight
    }

    /// Number of merged paths.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Resolves a symlink against this overlay's path map.
    pub fn resolve_link(&self, link_path: &str, target: &str) -> Option<(String, EntryKind)> {
        let mut current = normalize_link_target(parent_directory(link_path), target)?;
        for _ in 0..MAX_SYMLINK_DEPTH {
            match self.kind_at(&current)? {
                EntryKind::Symlink => {
                    let next = self
                        .nodes
                        .get(current.as_str())?
                        .entry
                        .link_target
                        .as_deref()?;
                    current = normalize_link_target(parent_directory(&current), next)?;
                }
                kind => return Some((current, kind)),
            }
        }
        None
    }

    /// The kind at `path`, recognizing directories that exist only as a prefix
    /// of deeper entries.
    fn kind_at(&self, path: &str) -> Option<EntryKind> {
        if let Some(node) = self.nodes.get(path) {
            return Some(node.entry.kind);
        }
        if path.is_empty() {
            return Some(EntryKind::Dir);
        }
        let prefix = format!("{path}/");
        self.nodes
            .keys()
            .any(|candidate| candidate.starts_with(&prefix))
            .then_some(EntryKind::Dir)
    }
}

/// Estimated weight of one node map, charging each path, its `Arc` header, the
/// map slot and the inline node fields plus symlink bytes.
fn node_map_weight(nodes: &im::HashMap<PathRef, Node>) -> usize {
    nodes
        .iter()
        .map(|(path, node)| {
            path.len()
                + ARC_STR_HEADER_BYTES
                + std::mem::size_of::<Node>()
                + HASH_SLOT_BYTES
                + node.entry.link_target.as_deref().map_or(0, str::len)
        })
        .sum()
}

/// Per-path change classification of a cumulative overlay against the overlay
/// one layer below it. Holds an entry for every changed non-directory path and
/// every directory whose descendant rollup is non-null.
#[derive(Debug, Default)]
pub struct Changes {
    changes: HashMap<String, Change>,
}

impl Changes {
    pub fn get(&self, path: &str) -> Option<Change> {
        self.changes.get(path).copied()
    }

    /// Estimated heap footprint of this change map in bytes.
    pub fn weight(&self) -> usize {
        self.changes
            .keys()
            .map(|path| path.len() + ENTRY_OVERHEAD_BYTES)
            .sum()
    }

    /// Number of classified paths.
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Classifies every path in `final_layer` relative to `lower_layer`. Directory
/// colour is derived from descendants, never from a synthesized directory's
/// placeholder mode.
pub fn classify(final_layer: &ComposedLayer, lower_layer: &ComposedLayer) -> Changes {
    let mut dirs: HashSet<String> = HashSet::new();
    ancestor_dirs(
        final_layer
            .iter()
            .map(|(path, node)| (path, node.entry.kind == EntryKind::Dir)),
        &mut dirs,
    );
    ancestor_dirs(
        lower_layer
            .iter()
            .map(|(path, node)| (path, node.entry.kind == EntryKind::Dir)),
        &mut dirs,
    );

    let mut lower_dirs: HashSet<String> = HashSet::new();
    ancestor_dirs(
        lower_layer
            .iter()
            .map(|(path, node)| (path, node.entry.kind == EntryKind::Dir)),
        &mut lower_dirs,
    );

    let mut final_desc: HashSet<String> = HashSet::new();
    for (path, _) in final_layer.iter() {
        add_ancestors(path, &mut final_desc);
    }
    let mut lower_desc: HashSet<String> = HashSet::new();
    for (path, _) in lower_layer.iter() {
        add_ancestors(path, &mut lower_desc);
    }

    let mut changes: HashMap<String, Change> = HashMap::new();

    // A path that is a directory in one layer and a leaf in the other is a type
    // modification; it never participates in directory rollup.
    let mut conflicts: HashSet<String> = HashSet::new();
    for (path, final_node) in final_layer.iter() {
        if let Some(lower_node) = lower_layer.get(path) {
            if (final_node.entry.kind == EntryKind::Dir)
                != (lower_node.entry.kind == EntryKind::Dir)
            {
                changes.insert(path.to_string(), Change::Modified);
                conflicts.insert(path.to_string());
            }
        }
    }
    for path in &conflicts {
        dirs.remove(path);
    }

    for (path, final_node) in final_layer.iter() {
        if conflicts.contains(path) || final_node.entry.kind == EntryKind::Dir {
            continue;
        }
        match lower_layer.get(path) {
            None => {
                changes.insert(path.to_string(), Change::New);
            }
            Some(lower_node) => {
                if !same_entry(&final_node.entry, &lower_node.entry) {
                    changes.insert(path.to_string(), Change::Modified);
                }
            }
        }
    }
    for (path, lower_node) in lower_layer.iter() {
        if conflicts.contains(path) || final_layer.get(path).is_some() {
            continue;
        }
        if lower_node.entry.kind == EntryKind::Dir {
            continue;
        }
        changes.insert(path.to_string(), Change::Removed);
    }

    let mut any_change: HashSet<String> = HashSet::new();
    for path in changes.keys() {
        add_ancestors(path, &mut any_change);
    }

    // Deepest directories first, so a child directory's rollup is known before
    // its parent aggregates it. Same-depth directories are never ancestors of
    // one another, so their relative order does not matter.
    let mut ordered: Vec<&String> = dirs.iter().collect();
    ordered.sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
    for path in ordered {
        let final_d = final_desc.contains(path);
        let lower_d = lower_desc.contains(path);
        let change = if lower_dirs.contains(path) && !final_d && final_layer.get(path).is_none() {
            Some(Change::Removed)
        } else if final_d && !lower_d {
            Some(Change::New)
        } else if any_change.contains(path) {
            Some(Change::Modified)
        } else {
            None
        };
        if let Some(change) = change {
            changes.insert(path.clone(), change);
            add_ancestors(path, &mut any_change);
        }
    }

    Changes { changes }
}

/// Recursive uncompressed byte total of every direct child directory of
/// `path`, computed in one pass over `entries`. A non-directory entry with a
/// positive size is folded into the child directory it lives under; an entry
/// at `path` itself or outside it contributes nothing. A child with no sized
/// descendants is absent, so callers read a missing name as `0`.
pub fn child_dir_sizes<'a>(
    entries: impl Iterator<Item = (&'a str, &'a CachedEntry)>,
    path: &str,
) -> HashMap<String, i64> {
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path}/")
    };
    let mut child_sizes: HashMap<String, i64> = HashMap::new();
    for (entry_path, entry) in entries {
        if entry.kind == EntryKind::Dir || entry.size <= 0 {
            continue;
        }
        let rest = if path.is_empty() {
            entry_path
        } else if let Some(rest) = entry_path.strip_prefix(&prefix) {
            rest
        } else {
            continue;
        };
        let Some((head, _)) = rest.split_once('/') else {
            continue;
        };
        let total = child_sizes.entry(head.to_string()).or_insert(0);
        *total = total.saturating_add(entry.size);
    }
    child_sizes
}

fn parent_directory(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((parent, _)) => parent,
        None => "",
    }
}

/// Joins `target` onto `base` (an absolute target restarts at the layer root)
/// and normalizes `.`/`..`, rejecting traversal above the root.
fn normalize_link_target(base: &str, target: &str) -> Option<String> {
    let mut components: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        base.split('/').filter(|part| !part.is_empty()).collect()
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            other => components.push(other),
        }
    }
    Some(components.join("/"))
}

/// Tunables for [`LayerIndexCache`].
#[derive(Debug, Clone, Copy)]
pub struct LayerCacheConfig {
    /// Total estimated bytes of cached indices before LRU eviction begins.
    pub max_bytes: usize,
    /// How long an index may be served before it is rebuilt.
    pub ttl: Duration,
}

impl LayerCacheConfig {
    /// 256 MiB: several hundred typical image layers, or a handful of layers at
    /// the 100k-entry cap, while staying well inside a small container's RAM.
    pub const DEFAULT_MAX_BYTES: usize = 256 * 1024 * 1024;
    /// 15 minutes: warm for an interactive browsing session, bounded staleness.
    pub const DEFAULT_TTL: Duration = Duration::from_secs(15 * 60);
}

impl Default for LayerCacheConfig {
    fn default() -> Self {
        Self {
            max_bytes: Self::DEFAULT_MAX_BYTES,
            ttl: Self::DEFAULT_TTL,
        }
    }
}

struct CacheEntry {
    index: Arc<LayerIndex>,
    inserted_at: Instant,
    /// LRU sequence number; larger means more recently used.
    seq: u64,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<Digest, CacheEntry>,
    /// `seq -> digest`, ordered so the first key is the LRU victim.
    order: BTreeMap<u64, Digest>,
    /// Per-digest build locks, so concurrent misses scan only once.
    inflight: HashMap<Digest, Arc<tokio::sync::Mutex<()>>>,
    clock: u64,
    weight: usize,
    /// The earliest `inserted_at + ttl` in `entries`; a sweep is due at or
    /// after it. Kept stale-too-early at worst (removals do not recompute it),
    /// which costs one no-op sweep, never a missed expiry.
    next_expires: Option<Instant>,
}

impl Inner {
    fn remove(&mut self, digest: &Digest) -> Option<Arc<LayerIndex>> {
        let entry = self.entries.remove(digest)?;
        self.order.remove(&entry.seq);
        self.weight = self.weight.saturating_sub(entry.index.weight());
        Some(entry.index)
    }

    /// Marks `digest` as most-recently-used. Returns the new sequence number.
    fn touch(&mut self, digest: &Digest) -> Option<u64> {
        let old_seq = self.entries.get(digest)?.seq;
        self.clock += 1;
        let next = self.clock;
        self.order.remove(&old_seq);
        self.order.insert(next, digest.clone());
        if let Some(entry) = self.entries.get_mut(digest) {
            entry.seq = next;
        }
        Some(next)
    }
}

/// RAII lease on one digest's in-flight build lock.
///
/// The lease removes `digest` from `inflight` when it is dropped, so a request
/// cancelled while waiting for or running a build — for example because the
/// client disconnected — cannot leave the entry behind forever. It removes the
/// entry only once no other caller still shares the same lock, so waiting
/// callers keep collapsing onto one build; the last holder removes it.
struct LayerBuildLease<'a> {
    cache: &'a LayerIndexCache,
    digest: Digest,
    slot: Arc<tokio::sync::Mutex<()>>,
}

impl LayerBuildLease<'_> {
    async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.slot.lock().await
    }
}

impl Drop for LayerBuildLease<'_> {
    fn drop(&mut self) {
        let mut inner = self.cache.lock();
        let ours = inner
            .inflight
            .get(&self.digest)
            .is_some_and(|existing| Arc::ptr_eq(existing, &self.slot));
        // `strong_count` is the map's reference plus one per live lease. While
        // the cache lock is held no lease can be created or dropped, so the
        // count is stable; leave the entry for any remaining waiters.
        if ours && Arc::strong_count(&self.slot) <= 2 {
            inner.inflight.remove(&self.digest);
        }
    }
}

/// Concurrent, bounded, self-expiring cache of layer indices.
pub struct LayerIndexCache {
    config: LayerCacheConfig,
    inner: Mutex<Inner>,
    /// Number of expired-entry sweeps performed.
    sweeps: AtomicU64,
    /// Bumped whenever an index is invalidated, so caches derived from indices
    /// (the composed-overlay cache) can refuse entries built before the change.
    generation: Arc<AtomicU64>,
    /// Woken when the generation advances, so an idle maintenance task can drop
    /// newly-stale derived entries without waiting for a TTL deadline.
    changed: Arc<tokio::sync::Notify>,
}

impl LayerIndexCache {
    pub fn new(config: LayerCacheConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(Inner::default()),
            sweeps: AtomicU64::new(0),
            generation: Arc::new(AtomicU64::new(0)),
            changed: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// The shared generation counter, incremented by [`Self::invalidate`] and
    /// [`Self::clear`]. A derived cache checks it to avoid serving data built
    /// from an index that has since been dropped.
    pub fn generation(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.generation)
    }

    /// A handle woken whenever the generation advances (see
    /// [`Self::invalidate`] and [`Self::clear`]).
    pub fn changed(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.changed)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Returns a cached index, dropping it first when its TTL has elapsed.
    pub fn get(&self, digest: &Digest) -> Option<Arc<LayerIndex>> {
        let now = Instant::now();
        let mut inner = self.lock();
        self.sweep_if_due(&mut inner, now);
        if self.is_expired(inner.entries.get(digest), now) {
            inner.remove(digest);
            return None;
        }
        inner.touch(digest);
        inner
            .entries
            .get(digest)
            .map(|entry| Arc::clone(&entry.index))
    }

    /// Stores an index, evicting least-recently-used entries to stay in budget.
    ///
    /// An index whose own weight exceeds the whole budget is skipped, so one
    /// oversized layer cannot flush every other cached layer.
    pub fn insert(&self, digest: Digest, index: Arc<LayerIndex>) {
        let weight = index.weight();
        if weight > self.config.max_bytes {
            tracing::debug!(digest = %digest, weight, "layer index exceeds cache budget; not cached");
            return;
        }

        let mut inner = self.lock();
        inner.remove(&digest);
        inner.clock += 1;
        let seq = inner.clock;
        let inserted_at = Instant::now();
        let expires = inserted_at + self.config.ttl;
        inner.weight += weight;
        inner.entries.insert(
            digest.clone(),
            CacheEntry {
                index,
                inserted_at,
                seq,
            },
        );
        inner.order.insert(seq, digest);
        inner.next_expires = Some(match inner.next_expires {
            Some(current) => current.min(expires),
            None => expires,
        });
        self.evict_to_budget(&mut inner);
    }

    /// Returns the cached index for `digest`, building it with `build` on a
    /// miss.
    ///
    /// A miss runs `build` on the blocking pool while other callers for the
    /// same digest wait on that digest's build lock; once it completes they
    /// observe the freshly cached index instead of rescanning the layer.
    pub async fn get_or_build<F>(&self, digest: &Digest, build: F) -> ApiResult<Arc<LayerIndex>>
    where
        F: FnOnce() -> ApiResult<Arc<LayerIndex>> + Send + 'static,
    {
        if let Some(index) = self.get(digest) {
            return Ok(index);
        }

        let lease = self.begin_build(digest);
        let _guard = lease.lock().await;

        // Another caller may have populated the cache while we waited. Dropping
        // `lease` on return releases the in-flight entry.
        if let Some(index) = self.get(digest) {
            return Ok(index);
        }

        match tokio::task::spawn_blocking(build).await {
            Ok(Ok(index)) => {
                self.insert(digest.clone(), Arc::clone(&index));
                Ok(index)
            }
            Ok(Err(err)) => Err(err),
            Err(err) => {
                tracing::error!(error = %err, "layer index build panicked");
                Err(ApiError::internal("layer reader failed"))
            }
        }
    }

    /// Drops the cached index for `digest`, e.g. after its blob is removed, and
    /// advances the generation so no derived overlay serves pre-invalidation
    /// data.
    pub fn invalidate(&self, digest: &Digest) {
        self.lock().remove(digest);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_one();
    }

    /// Drops every cached index and advances the generation.
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.entries.clear();
        inner.order.clear();
        inner.weight = 0;
        inner.next_expires = None;
        drop(inner);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_one();
    }

    /// The instant the earliest cached index expires, or `None` when nothing is
    /// cached. A maintenance task sleeps until then instead of polling.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.lock().next_expires
    }

    /// Drops every index older than the TTL.
    pub fn sweep_expired(&self) {
        let now = Instant::now();
        let mut inner = self.lock();
        self.sweep_expired_locked(&mut inner, now);
    }

    /// Number of cached indices (introspection for tests and metrics).
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    /// Estimated total bytes of cached indices.
    pub fn weight(&self) -> usize {
        self.lock().weight
    }

    /// Number of expired-entry sweeps performed (introspection for tests and
    /// metrics).
    pub fn sweep_count(&self) -> u64 {
        self.sweeps.load(Ordering::SeqCst)
    }

    fn is_expired(&self, entry: Option<&CacheEntry>, now: Instant) -> bool {
        entry.is_some_and(|entry| now.duration_since(entry.inserted_at) >= self.config.ttl)
    }

    fn begin_build(&self, digest: &Digest) -> LayerBuildLease<'_> {
        let mut inner = self.lock();
        let slot = Arc::clone(
            inner
                .inflight
                .entry(digest.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        );
        LayerBuildLease {
            cache: self,
            digest: digest.clone(),
            slot,
        }
    }

    fn evict_to_budget(&self, inner: &mut Inner) {
        while inner.weight > self.config.max_bytes {
            let victim = inner.order.iter().next().map(|(_, digest)| digest.clone());
            match victim {
                Some(victim) => {
                    inner.remove(&victim);
                }
                None => break,
            }
        }
    }

    fn sweep_if_due(&self, inner: &mut Inner, now: Instant) {
        let due = inner.next_expires.is_some_and(|next| now >= next);
        if !due {
            return;
        }
        self.sweep_expired_locked(inner, now);
    }

    fn sweep_expired_locked(&self, inner: &mut Inner, now: Instant) {
        self.sweeps.fetch_add(1, Ordering::SeqCst);

        let ttl = self.config.ttl;
        let mut next: Option<Instant> = None;
        let mut expired: Vec<Digest> = Vec::new();
        for (digest, entry) in inner.entries.iter() {
            if now.duration_since(entry.inserted_at) >= ttl {
                expired.push(digest.clone());
            } else {
                let expires = entry.inserted_at + ttl;
                next = Some(next.map_or(expires, |current| current.min(expires)));
            }
        }
        for digest in &expired {
            inner.remove(digest);
        }
        inner.next_expires = next;
    }
}

/// Key of one cumulative overlay: the manifest it belongs to and the 0-based
/// position of the last layer included. The position is part of the key because
/// a manifest may repeat a layer digest; each position is a distinct aggregate.
pub type ComposedKey = (Digest, usize);

struct ComposedEntry {
    layer: Arc<ComposedLayer>,
    inserted_at: Instant,
    seq: u64,
    generation: u64,
}

#[derive(Default)]
struct ComposedInner {
    entries: HashMap<ComposedKey, ComposedEntry>,
    order: BTreeMap<u64, ComposedKey>,
    inflight: HashMap<ComposedKey, Arc<tokio::sync::Mutex<()>>>,
    clock: u64,
    weight: usize,
    next_expires: Option<Instant>,
}

impl ComposedInner {
    fn remove(&mut self, key: &ComposedKey) -> Option<Arc<ComposedLayer>> {
        let entry = self.entries.remove(key)?;
        self.order.remove(&entry.seq);
        self.weight = self.weight.saturating_sub(entry.layer.weight());
        Some(entry.layer)
    }

    fn touch(&mut self, key: &ComposedKey) -> Option<u64> {
        let old_seq = self.entries.get(key)?.seq;
        self.clock += 1;
        let next = self.clock;
        self.order.remove(&old_seq);
        self.order.insert(next, key.clone());
        if let Some(entry) = self.entries.get_mut(key) {
            entry.seq = next;
        }
        Some(next)
    }
}

/// RAII lease on one composed-overlay build lock; see [`LayerBuildLease`].
struct ComposedBuildLease<'a> {
    cache: &'a ComposedLayerCache,
    key: ComposedKey,
    slot: Arc<tokio::sync::Mutex<()>>,
}

impl ComposedBuildLease<'_> {
    async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.slot.lock().await
    }
}

impl Drop for ComposedBuildLease<'_> {
    fn drop(&mut self) {
        let mut inner = self.cache.lock();
        let ours = inner
            .inflight
            .get(&self.key)
            .is_some_and(|existing| Arc::ptr_eq(existing, &self.slot));
        // See `LayerBuildLease`: leave the entry for waiters that share it.
        if ours && Arc::strong_count(&self.slot) <= 2 {
            inner.inflight.remove(&self.key);
        }
    }
}

/// Concurrent, bounded, self-expiring cache of cumulative layer overlays.
///
/// Entries are keyed by `(manifest_digest, layer_position)` and stamped with
/// the [`LayerIndexCache`] generation at build time; a lookup whose generation
/// is stale is a miss, so invalidating a layer index can never leave a composed
/// overlay serving data derived from it.
pub struct ComposedLayerCache {
    config: LayerCacheConfig,
    inner: Mutex<ComposedInner>,
    sweeps: AtomicU64,
    generation: Arc<AtomicU64>,
}

impl ComposedLayerCache {
    pub fn new(config: LayerCacheConfig, generation: Arc<AtomicU64>) -> Self {
        Self {
            config,
            inner: Mutex::new(ComposedInner::default()),
            sweeps: AtomicU64::new(0),
            generation,
        }
    }

    fn lock(&self) -> MutexGuard<'_, ComposedInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Returns a cached overlay, dropping it first when its generation is stale
    /// or its TTL has elapsed.
    pub fn get(&self, key: &ComposedKey) -> Option<Arc<ComposedLayer>> {
        let now = Instant::now();
        let mut inner = self.lock();
        self.sweep_if_due(&mut inner, now);
        let generation = self.current_generation();
        if inner
            .entries
            .get(key)
            .is_some_and(|entry| entry.generation != generation)
        {
            inner.remove(key);
        }
        if self.is_expired(inner.entries.get(key), now) {
            inner.remove(key);
            return None;
        }
        inner.touch(key);
        inner.entries.get(key).map(|entry| Arc::clone(&entry.layer))
    }

    /// Stores an overlay, evicting least-recently-used entries to stay in
    /// budget.
    pub fn insert(&self, key: ComposedKey, layer: Arc<ComposedLayer>) {
        let generation = self.current_generation();
        self.insert_with_generation(key, layer, generation);
    }

    fn insert_with_generation(&self, key: ComposedKey, layer: Arc<ComposedLayer>, generation: u64) {
        let weight = layer.weight();
        if weight > self.config.max_bytes {
            tracing::debug!(weight, "composed overlay exceeds cache budget; not cached");
            return;
        }
        let mut inner = self.lock();
        inner.remove(&key);
        inner.clock += 1;
        let seq = inner.clock;
        let inserted_at = Instant::now();
        let expires = inserted_at + self.config.ttl;
        inner.weight += weight;
        inner.entries.insert(
            key.clone(),
            ComposedEntry {
                layer,
                inserted_at,
                seq,
                generation,
            },
        );
        inner.order.insert(seq, key);
        inner.next_expires = Some(match inner.next_expires {
            Some(current) => current.min(expires),
            None => expires,
        });
        self.evict_to_budget(&mut inner);
    }

    /// Returns the cached overlay for `key`, building it with `build` on a miss.
    pub async fn get_or_build<F>(
        &self,
        key: &ComposedKey,
        build: F,
    ) -> ApiResult<Arc<ComposedLayer>>
    where
        F: FnOnce() -> ApiResult<Arc<ComposedLayer>> + Send + 'static,
    {
        if let Some(layer) = self.get(key) {
            return Ok(layer);
        }

        let lease = self.begin_build(key);
        let _guard = lease.lock().await;

        if let Some(layer) = self.get(key) {
            return Ok(layer);
        }

        let generation = self.current_generation();
        match tokio::task::spawn_blocking(build).await {
            Ok(Ok(layer)) => {
                self.insert_with_generation(key.clone(), Arc::clone(&layer), generation);
                Ok(layer)
            }
            Ok(Err(err)) => Err(err),
            Err(err) => {
                tracing::error!(error = %err, "composed overlay build panicked");
                Err(ApiError::internal("layer reader failed"))
            }
        }
    }

    /// The instant the earliest cached overlay expires, or `None` when nothing
    /// is cached. A maintenance task sleeps until then instead of polling.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.lock().next_expires
    }

    /// Drops every overlay older than the TTL.
    pub fn sweep_expired(&self) {
        let now = Instant::now();
        let mut inner = self.lock();
        self.sweep_expired_locked(&mut inner, now);
    }

    /// Drops every overlay built from a layer-index generation that is no
    /// longer current. A lookup already treats such an entry as a miss; this
    /// reclaims the memory of entries that are never looked up again.
    pub fn sweep_stale_generation(&self) {
        let generation = self.current_generation();
        let mut inner = self.lock();
        let stale: Vec<ComposedKey> = inner
            .entries
            .iter()
            .filter(|(_, entry)| entry.generation != generation)
            .map(|(key, _)| key.clone())
            .collect();
        for key in &stale {
            inner.remove(key);
        }
    }

    /// Number of cached overlays (introspection for tests and metrics).
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    /// Estimated total bytes of cached overlays.
    pub fn weight(&self) -> usize {
        self.lock().weight
    }

    /// Number of expired-entry sweeps performed.
    pub fn sweep_count(&self) -> u64 {
        self.sweeps.load(Ordering::SeqCst)
    }

    fn is_expired(&self, entry: Option<&ComposedEntry>, now: Instant) -> bool {
        entry.is_some_and(|entry| now.duration_since(entry.inserted_at) >= self.config.ttl)
    }

    fn begin_build(&self, key: &ComposedKey) -> ComposedBuildLease<'_> {
        let mut inner = self.lock();
        let slot = Arc::clone(
            inner
                .inflight
                .entry(key.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        );
        ComposedBuildLease {
            cache: self,
            key: key.clone(),
            slot,
        }
    }

    fn evict_to_budget(&self, inner: &mut ComposedInner) {
        while inner.weight > self.config.max_bytes {
            let victim = inner.order.iter().next().map(|(_, key)| key.clone());
            match victim {
                Some(victim) => {
                    inner.remove(&victim);
                }
                None => break,
            }
        }
    }

    fn sweep_if_due(&self, inner: &mut ComposedInner, now: Instant) {
        let due = inner.next_expires.is_some_and(|next| now >= next);
        if !due {
            return;
        }
        self.sweep_expired_locked(inner, now);
    }

    fn sweep_expired_locked(&self, inner: &mut ComposedInner, now: Instant) {
        self.sweeps.fetch_add(1, Ordering::SeqCst);
        let ttl = self.config.ttl;
        let mut next: Option<Instant> = None;
        let mut expired: Vec<ComposedKey> = Vec::new();
        for (key, entry) in inner.entries.iter() {
            if now.duration_since(entry.inserted_at) >= ttl {
                expired.push(key.clone());
            } else {
                let expires = entry.inserted_at + ttl;
                next = Some(next.map_or(expires, |current| current.min(expires)));
            }
        }
        for key in &expired {
            inner.remove(key);
        }
        inner.next_expires = next;
    }
}

struct ChangesEntry {
    changes: Arc<Changes>,
    inserted_at: Instant,
    seq: u64,
    generation: u64,
}

#[derive(Default)]
struct ChangesInner {
    entries: HashMap<ComposedKey, ChangesEntry>,
    order: BTreeMap<u64, ComposedKey>,
    inflight: HashMap<ComposedKey, Arc<tokio::sync::Mutex<()>>>,
    clock: u64,
    weight: usize,
    next_expires: Option<Instant>,
}

impl ChangesInner {
    fn remove(&mut self, key: &ComposedKey) -> Option<Arc<Changes>> {
        let entry = self.entries.remove(key)?;
        self.order.remove(&entry.seq);
        self.weight = self.weight.saturating_sub(entry.changes.weight());
        Some(entry.changes)
    }

    fn touch(&mut self, key: &ComposedKey) -> Option<u64> {
        let old_seq = self.entries.get(key)?.seq;
        self.clock += 1;
        let next = self.clock;
        self.order.remove(&old_seq);
        self.order.insert(next, key.clone());
        if let Some(entry) = self.entries.get_mut(key) {
            entry.seq = next;
        }
        Some(next)
    }
}

/// RAII lease on one change-classification build lock; see [`LayerBuildLease`].
struct ChangesBuildLease<'a> {
    cache: &'a ChangesCache,
    key: ComposedKey,
    slot: Arc<tokio::sync::Mutex<()>>,
}

impl ChangesBuildLease<'_> {
    async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.slot.lock().await
    }
}

impl Drop for ChangesBuildLease<'_> {
    fn drop(&mut self) {
        let mut inner = self.cache.lock();
        let ours = inner
            .inflight
            .get(&self.key)
            .is_some_and(|existing| Arc::ptr_eq(existing, &self.slot));
        // See `LayerBuildLease`: leave the entry for waiters that share it.
        if ours && Arc::strong_count(&self.slot) <= 2 {
            inner.inflight.remove(&self.key);
        }
    }
}

/// Concurrent, bounded, self-expiring cache of diff classifications.
///
/// Entries are keyed by `(manifest_digest, layer_position)` and stamped with
/// the [`LayerIndexCache`] generation at build time; a lookup whose generation
/// is stale is a miss, so invalidating a layer index can never leave a
/// classification of data derived from it.
pub struct ChangesCache {
    config: LayerCacheConfig,
    inner: Mutex<ChangesInner>,
    sweeps: AtomicU64,
    generation: Arc<AtomicU64>,
}

impl ChangesCache {
    pub fn new(config: LayerCacheConfig, generation: Arc<AtomicU64>) -> Self {
        Self {
            config,
            inner: Mutex::new(ChangesInner::default()),
            sweeps: AtomicU64::new(0),
            generation,
        }
    }

    fn lock(&self) -> MutexGuard<'_, ChangesInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Returns a cached classification, dropping it first when its generation
    /// is stale or its TTL has elapsed.
    pub fn get(&self, key: &ComposedKey) -> Option<Arc<Changes>> {
        let now = Instant::now();
        let mut inner = self.lock();
        self.sweep_if_due(&mut inner, now);
        let generation = self.current_generation();
        if inner
            .entries
            .get(key)
            .is_some_and(|entry| entry.generation != generation)
        {
            inner.remove(key);
        }
        if self.is_expired(inner.entries.get(key), now) {
            inner.remove(key);
            return None;
        }
        inner.touch(key);
        inner
            .entries
            .get(key)
            .map(|entry| Arc::clone(&entry.changes))
    }

    /// Stores a classification, evicting least-recently-used entries to stay in
    /// budget.
    pub fn insert(&self, key: ComposedKey, changes: Arc<Changes>) {
        let generation = self.current_generation();
        self.insert_with_generation(key, changes, generation);
    }

    fn insert_with_generation(&self, key: ComposedKey, changes: Arc<Changes>, generation: u64) {
        let weight = changes.weight();
        if weight > self.config.max_bytes {
            tracing::debug!(weight, "layer change map exceeds cache budget; not cached");
            return;
        }
        let mut inner = self.lock();
        inner.remove(&key);
        inner.clock += 1;
        let seq = inner.clock;
        let inserted_at = Instant::now();
        let expires = inserted_at + self.config.ttl;
        inner.weight += weight;
        inner.entries.insert(
            key.clone(),
            ChangesEntry {
                changes,
                inserted_at,
                seq,
                generation,
            },
        );
        inner.order.insert(seq, key);
        inner.next_expires = Some(match inner.next_expires {
            Some(current) => current.min(expires),
            None => expires,
        });
        self.evict_to_budget(&mut inner);
    }

    /// Returns the cached classification for `key`, building it with `build` on
    /// a miss.
    pub async fn get_or_build<F>(&self, key: &ComposedKey, build: F) -> ApiResult<Arc<Changes>>
    where
        F: FnOnce() -> ApiResult<Arc<Changes>> + Send + 'static,
    {
        if let Some(changes) = self.get(key) {
            return Ok(changes);
        }

        let lease = self.begin_build(key);
        let _guard = lease.lock().await;

        if let Some(changes) = self.get(key) {
            return Ok(changes);
        }

        let generation = self.current_generation();
        match tokio::task::spawn_blocking(build).await {
            Ok(Ok(changes)) => {
                self.insert_with_generation(key.clone(), Arc::clone(&changes), generation);
                Ok(changes)
            }
            Ok(Err(err)) => Err(err),
            Err(err) => {
                tracing::error!(error = %err, "layer change classification panicked");
                Err(ApiError::internal("layer reader failed"))
            }
        }
    }

    /// The instant the earliest cached classification expires, or `None` when
    /// nothing is cached. A maintenance task sleeps until then instead of
    /// polling.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.lock().next_expires
    }

    /// Drops every classification older than the TTL.
    pub fn sweep_expired(&self) {
        let now = Instant::now();
        let mut inner = self.lock();
        self.sweep_expired_locked(&mut inner, now);
    }

    /// Drops every classification built from a layer-index generation that is
    /// no longer current. A lookup already treats such an entry as a miss; this
    /// reclaims the memory of entries that are never looked up again.
    pub fn sweep_stale_generation(&self) {
        let generation = self.current_generation();
        let mut inner = self.lock();
        let stale: Vec<ComposedKey> = inner
            .entries
            .iter()
            .filter(|(_, entry)| entry.generation != generation)
            .map(|(key, _)| key.clone())
            .collect();
        for key in &stale {
            inner.remove(key);
        }
    }

    /// Number of cached classifications (introspection for tests and metrics).
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    /// Estimated total bytes of cached classifications.
    pub fn weight(&self) -> usize {
        self.lock().weight
    }

    /// Number of expired-entry sweeps performed.
    pub fn sweep_count(&self) -> u64 {
        self.sweeps.load(Ordering::SeqCst)
    }

    fn is_expired(&self, entry: Option<&ChangesEntry>, now: Instant) -> bool {
        entry.is_some_and(|entry| now.duration_since(entry.inserted_at) >= self.config.ttl)
    }

    fn begin_build(&self, key: &ComposedKey) -> ChangesBuildLease<'_> {
        let mut inner = self.lock();
        let slot = Arc::clone(
            inner
                .inflight
                .entry(key.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        );
        ChangesBuildLease {
            cache: self,
            key: key.clone(),
            slot,
        }
    }

    fn evict_to_budget(&self, inner: &mut ChangesInner) {
        while inner.weight > self.config.max_bytes {
            let victim = inner.order.iter().next().map(|(_, key)| key.clone());
            match victim {
                Some(victim) => {
                    inner.remove(&victim);
                }
                None => break,
            }
        }
    }

    fn sweep_if_due(&self, inner: &mut ChangesInner, now: Instant) {
        let due = inner.next_expires.is_some_and(|next| now >= next);
        if !due {
            return;
        }
        self.sweep_expired_locked(inner, now);
    }

    fn sweep_expired_locked(&self, inner: &mut ChangesInner, now: Instant) {
        self.sweeps.fetch_add(1, Ordering::SeqCst);
        let ttl = self.config.ttl;
        let mut next: Option<Instant> = None;
        let mut expired: Vec<ComposedKey> = Vec::new();
        for (key, entry) in inner.entries.iter() {
            if now.duration_since(entry.inserted_at) >= ttl {
                expired.push(key.clone());
            } else {
                let expires = entry.inserted_at + ttl;
                next = Some(next.map_or(expires, |current| current.min(expires)));
            }
        }
        for key in &expired {
            inner.remove(key);
        }
        inner.next_expires = next;
    }
}

/// Key of one rendered directory listing: which manifest/layer it belongs to,
/// which layer position it was rendered at, the requested path, and the tree
/// mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ListingKey {
    pub scope: String,
    pub position: u32,
    pub path: String,
    pub mode: &'static str,
}

struct ListingEntry {
    body: Bytes,
    inserted_at: Instant,
    seq: u64,
    generation: u64,
}

#[derive(Default)]
struct ListingInner {
    entries: HashMap<ListingKey, ListingEntry>,
    order: BTreeMap<u64, ListingKey>,
    clock: u64,
    weight: usize,
    next_expires: Option<Instant>,
}

impl ListingInner {
    fn remove(&mut self, key: &ListingKey) -> Option<Bytes> {
        let entry = self.entries.remove(key)?;
        self.order.remove(&entry.seq);
        self.weight = self.weight.saturating_sub(listing_weight(key, &entry.body));
        Some(entry.body)
    }

    fn touch(&mut self, key: &ListingKey) -> Option<u64> {
        let old_seq = self.entries.get(key)?.seq;
        self.clock += 1;
        let next = self.clock;
        self.order.remove(&old_seq);
        self.order.insert(next, key.clone());
        if let Some(entry) = self.entries.get_mut(key) {
            entry.seq = next;
        }
        Some(next)
    }
}

/// Concurrent, bounded, self-expiring cache of rendered directory listings.
///
/// Entries hold the already-serialized JSON body keyed by the requested
/// listing, and are stamped with the [`LayerIndexCache`] generation at build
/// time; a lookup whose generation is stale is a miss, so invalidating a layer
/// index can never leave a listing rendered from it.
pub struct ListingCache {
    config: LayerCacheConfig,
    inner: Mutex<ListingInner>,
    sweeps: AtomicU64,
    generation: Arc<AtomicU64>,
}

impl ListingCache {
    pub fn new(config: LayerCacheConfig, generation: Arc<AtomicU64>) -> Self {
        Self {
            config,
            inner: Mutex::new(ListingInner::default()),
            sweeps: AtomicU64::new(0),
            generation,
        }
    }

    fn lock(&self) -> MutexGuard<'_, ListingInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Returns a cached listing body, dropping it first when its generation is
    /// stale or its TTL has elapsed.
    pub fn get(&self, key: &ListingKey) -> Option<Bytes> {
        let now = Instant::now();
        let mut inner = self.lock();
        self.sweep_if_due(&mut inner, now);
        let generation = self.current_generation();
        if inner
            .entries
            .get(key)
            .is_some_and(|entry| entry.generation != generation)
        {
            inner.remove(key);
        }
        if self.is_expired(inner.entries.get(key), now) {
            inner.remove(key);
            return None;
        }
        inner.touch(key);
        inner.entries.get(key).map(|entry| entry.body.clone())
    }

    /// Stores a rendered listing body, evicting least-recently-used entries to
    /// stay in budget.
    pub fn insert(&self, key: ListingKey, body: Bytes) {
        let generation = self.current_generation();
        self.insert_with_generation(key, body, generation);
    }

    /// The current layer-index generation, so a caller can capture it before a
    /// build and keep a listing computed from an index invalidated mid-build
    /// from being served as fresh.
    pub fn generation_token(&self) -> u64 {
        self.current_generation()
    }

    /// Stores a listing body stamped with `generation` rather than the current
    /// one; see [`Self::generation_token`].
    pub fn insert_with_generation(&self, key: ListingKey, body: Bytes, generation: u64) {
        let weight = listing_weight(&key, &body);
        if weight > self.config.max_bytes {
            tracing::debug!(weight, "layer listing exceeds cache budget; not cached");
            return;
        }
        let mut inner = self.lock();
        inner.remove(&key);
        inner.clock += 1;
        let seq = inner.clock;
        let inserted_at = Instant::now();
        let expires = inserted_at + self.config.ttl;
        inner.weight += weight;
        inner.entries.insert(
            key.clone(),
            ListingEntry {
                body,
                inserted_at,
                seq,
                generation,
            },
        );
        inner.order.insert(seq, key);
        inner.next_expires = Some(match inner.next_expires {
            Some(current) => current.min(expires),
            None => expires,
        });
        self.evict_to_budget(&mut inner);
    }

    /// The instant the earliest cached listing expires, or `None` when nothing
    /// is cached. A maintenance task sleeps until then instead of polling.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.lock().next_expires
    }

    /// Drops every listing older than the TTL.
    pub fn sweep_expired(&self) {
        let now = Instant::now();
        let mut inner = self.lock();
        self.sweep_expired_locked(&mut inner, now);
    }

    /// Drops every listing built from a layer-index generation that is no
    /// longer current. A lookup already treats such an entry as a miss; this
    /// reclaims the memory of entries that are never looked up again.
    pub fn sweep_stale_generation(&self) {
        let generation = self.current_generation();
        let mut inner = self.lock();
        let stale: Vec<ListingKey> = inner
            .entries
            .iter()
            .filter(|(_, entry)| entry.generation != generation)
            .map(|(key, _)| key.clone())
            .collect();
        for key in &stale {
            inner.remove(key);
        }
    }

    /// Number of cached listings (introspection for tests and metrics).
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    /// Estimated total bytes of cached listings.
    pub fn weight(&self) -> usize {
        self.lock().weight
    }

    /// Number of expired-entry sweeps performed.
    pub fn sweep_count(&self) -> u64 {
        self.sweeps.load(Ordering::SeqCst)
    }

    fn is_expired(&self, entry: Option<&ListingEntry>, now: Instant) -> bool {
        entry.is_some_and(|entry| now.duration_since(entry.inserted_at) >= self.config.ttl)
    }

    fn evict_to_budget(&self, inner: &mut ListingInner) {
        while inner.weight > self.config.max_bytes {
            let victim = inner.order.iter().next().map(|(_, key)| key.clone());
            match victim {
                Some(victim) => {
                    inner.remove(&victim);
                }
                None => break,
            }
        }
    }

    fn sweep_if_due(&self, inner: &mut ListingInner, now: Instant) {
        let due = inner.next_expires.is_some_and(|next| now >= next);
        if !due {
            return;
        }
        self.sweep_expired_locked(inner, now);
    }

    fn sweep_expired_locked(&self, inner: &mut ListingInner, now: Instant) {
        self.sweeps.fetch_add(1, Ordering::SeqCst);
        let ttl = self.config.ttl;
        let mut next: Option<Instant> = None;
        let mut expired: Vec<ListingKey> = Vec::new();
        for (key, entry) in inner.entries.iter() {
            if now.duration_since(entry.inserted_at) >= ttl {
                expired.push(key.clone());
            } else {
                let expires = entry.inserted_at + ttl;
                next = Some(next.map_or(expires, |current| current.min(expires)));
            }
        }
        for key in &expired {
            inner.remove(key);
        }
        inner.next_expires = next;
    }
}

/// Estimated weight of one cached listing: the serialized body plus the key's
/// owned strings and fixed overhead.
fn listing_weight(key: &ListingKey, body: &Bytes) -> usize {
    body.len() + key.scope.len() + key.path.len() + key.mode.len() + LISTING_KEY_OVERHEAD_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn digest(seed: u8) -> Digest {
        Digest::from_bytes(&[seed])
    }

    fn index(paths: &[&str]) -> Arc<LayerIndex> {
        let mut entries = HashMap::new();
        for path in paths {
            entries.insert((*path).to_string(), CachedEntry::directory());
        }
        Arc::new(LayerIndex::from_entries(entries))
    }

    fn file_entry(path: &str, size: i64) -> (String, CachedEntry) {
        (
            path.to_string(),
            CachedEntry {
                kind: EntryKind::File,
                size,
                mode: 0o644,
                link_target: None,
            },
        )
    }

    fn symlink(path: &str, target: &str) -> (String, CachedEntry) {
        (
            path.to_string(),
            CachedEntry {
                kind: EntryKind::Symlink,
                size: 0,
                mode: 0o777,
                link_target: Some(target.to_string().into_boxed_str()),
            },
        )
    }

    #[test]
    fn child_directory_sizes_are_summed_recursively() {
        let entries: HashMap<String, CachedEntry> = [
            file_entry("etc/config", 5),
            file_entry("usr/bin/run", 10),
            file_entry("usr/lib/lib.so", 20),
            ("usr".to_string(), CachedEntry::directory()),
        ]
        .into_iter()
        .collect();
        let sizes = |dir: &str| {
            child_dir_sizes(
                entries.iter().map(|(path, entry)| (path.as_str(), entry)),
                dir,
            )
        };

        assert_eq!(sizes("").get("etc").copied().unwrap_or(0), 5);
        assert_eq!(
            sizes("").get("usr").copied().unwrap_or(0),
            30,
            "nested files roll up"
        );
        assert_eq!(sizes("usr").get("bin").copied().unwrap_or(0), 10);
        assert_eq!(sizes("usr").get("lib").copied().unwrap_or(0), 20);
        assert_eq!(
            sizes("").get("var").copied().unwrap_or(0),
            0,
            "unknown directory"
        );
    }

    #[test]
    fn resolves_symlinks_to_files_and_directories() {
        let entries: HashMap<String, CachedEntry> = [
            file_entry("etc/config", 5),
            file_entry("usr/bin/run", 10),
            symlink("usr/bin/link", "run"),
            symlink("usr/abs", "/usr/bin/run"),
            symlink("usr/up", "../etc"),
            symlink("root-link", "usr"),
            symlink("usr/bin/chain", "link"),
            symlink("dangling", "missing"),
            symlink("loop-a", "loop-b"),
            symlink("loop-b", "loop-a"),
            symlink("escape", "../../etc/passwd"),
        ]
        .into_iter()
        .collect();
        let index = LayerIndex::from_entries(entries);

        assert_eq!(
            index.resolve_link("usr/bin/link", "run"),
            Some(("usr/bin/run".to_string(), EntryKind::File)),
            "relative file target"
        );
        assert_eq!(
            index.resolve_link("usr/abs", "/usr/bin/run"),
            Some(("usr/bin/run".to_string(), EntryKind::File)),
            "absolute target"
        );
        assert_eq!(
            index.resolve_link("usr/up", "../etc"),
            Some(("etc".to_string(), EntryKind::Dir)),
            "parent traversal"
        );
        assert_eq!(
            index.resolve_link("root-link", "usr"),
            Some(("usr".to_string(), EntryKind::Dir)),
            "synthesized directory target"
        );
        assert_eq!(
            index.resolve_link("usr/bin/chain", "link"),
            Some(("usr/bin/run".to_string(), EntryKind::File)),
            "chain follows to the final file"
        );
        assert_eq!(index.resolve_link("dangling", "missing"), None, "dangling");
        assert_eq!(index.resolve_link("loop-a", "loop-b"), None, "cycle");
        assert_eq!(
            index.resolve_link("escape", "../../etc/passwd"),
            None,
            "traversal above root"
        );
    }

    fn config(max_bytes: usize) -> LayerCacheConfig {
        LayerCacheConfig {
            max_bytes,
            ttl: LayerCacheConfig::DEFAULT_TTL,
        }
    }

    #[test]
    fn insert_then_get_returns_the_same_index() {
        let cache = LayerIndexCache::new(config(1024));
        let a = digest(1);
        let built = index(&["etc", "usr"]);
        cache.insert(a.clone(), Arc::clone(&built));

        let fetched = cache.get(&a).expect("cache hit");
        assert!(Arc::ptr_eq(&fetched, &built));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.weight(), built.weight());
    }

    #[test]
    fn evicts_the_least_recently_used_index_over_budget() {
        // Each single-path index weighs 64 + len, so three of them exceed 150.
        let cache = LayerIndexCache::new(config(150));
        let a = digest(1);
        let b = digest(2);
        let c = digest(3);
        cache.insert(a.clone(), index(&["a"]));
        cache.insert(b.clone(), index(&["b"]));
        assert_eq!(cache.len(), 2, "two indices fit inside the budget");

        // Touching `a` makes `b` the least-recently-used victim.
        assert!(cache.get(&a).is_some());
        cache.insert(c.clone(), index(&["cc"]));

        assert_eq!(cache.len(), 2);
        assert!(cache.get(&b).is_none(), "b was evicted");
        assert!(cache.get(&a).is_some(), "a was recently used");
        assert!(cache.get(&c).is_some(), "c was just inserted");
    }

    #[test]
    fn an_index_larger_than_the_budget_is_not_cached() {
        let cache = LayerIndexCache::new(config(100));
        cache.insert(digest(1), index(&["a", "b", "c"]));
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.weight(), 0);
    }

    #[test]
    fn an_expired_index_is_treated_as_a_miss() {
        let cache = LayerIndexCache::new(LayerCacheConfig {
            max_bytes: 1024,
            ttl: Duration::from_millis(10),
        });
        let a = digest(1);
        cache.insert(a.clone(), index(&["etc"]));
        assert!(cache.get(&a).is_some());

        std::thread::sleep(Duration::from_millis(20));
        assert!(cache.get(&a).is_none(), "TTL elapsed");
        assert_eq!(cache.len(), 0, "expired entry is reclaimed");
    }

    #[test]
    fn sweep_runs_only_when_the_earliest_entry_is_due() {
        let cache = LayerIndexCache::new(LayerCacheConfig {
            max_bytes: 1024,
            ttl: Duration::from_millis(400),
        });
        let a = digest(1);
        cache.insert(a.clone(), index(&["etc"]));

        for _ in 0..20 {
            assert!(cache.get(&a).is_some());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(cache.sweep_count(), 0, "no sweep before the deadline");

        std::thread::sleep(Duration::from_millis(420));
        assert!(cache.get(&a).is_none(), "expired at the deadline");
        assert_eq!(cache.sweep_count(), 1, "exactly one sweep at the deadline");

        assert!(cache.get(&a).is_none());
        assert_eq!(cache.sweep_count(), 1, "nothing left to sweep");
    }

    #[test]
    fn next_expires_tracks_the_earliest_inserted_entry() {
        let cache = LayerIndexCache::new(LayerCacheConfig {
            max_bytes: 4096,
            ttl: Duration::from_millis(300),
        });
        let a = digest(1);
        let b = digest(2);
        cache.insert(a.clone(), index(&["a"]));
        std::thread::sleep(Duration::from_millis(120));
        cache.insert(b.clone(), index(&["b"]));

        // A read of the newer entry must still sweep the older one once its
        // deadline passes; a "restart to newest" deadline would leave it.
        std::thread::sleep(Duration::from_millis(220));
        assert!(cache.get(&b).is_some(), "b is still fresh");
        assert_eq!(cache.sweep_count(), 1);
        assert_eq!(cache.len(), 1, "a was swept, b remains");
        assert!(cache.get(&a).is_none());
    }

    #[test]
    fn invalidate_and_clear_drop_entries() {
        let cache = LayerIndexCache::new(config(1024));
        let a = digest(1);
        let b = digest(2);
        cache.insert(a.clone(), index(&["a"]));
        cache.insert(b, index(&["b"]));

        cache.invalidate(&a);
        assert!(cache.get(&a).is_none());
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.weight(), 0);
    }

    #[tokio::test]
    async fn get_or_build_caches_the_result() {
        let cache = LayerIndexCache::new(config(1024));
        let a = digest(1);
        let calls = Arc::new(AtomicUsize::new(0));

        let first = {
            let calls = Arc::clone(&calls);
            cache
                .get_or_build(&a, move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(index(&["etc"]))
                })
                .await
                .expect("build")
        };
        let second = cache
            .get_or_build(&a, || -> ApiResult<Arc<LayerIndex>> {
                panic!("must not rebuild a cached index")
            })
            .await
            .expect("cache hit");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn concurrent_misses_share_one_build() {
        let cache = Arc::new(LayerIndexCache::new(config(1024)));
        let a = digest(1);
        let calls = Arc::new(AtomicUsize::new(0));

        let spawn = |cache: Arc<LayerIndexCache>, calls: Arc<AtomicUsize>, a: Digest| {
            tokio::spawn(async move {
                cache
                    .get_or_build(&a, move || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(50));
                        Ok(index(&["etc"]))
                    })
                    .await
            })
        };

        let (first, second) = tokio::join!(
            spawn(Arc::clone(&cache), Arc::clone(&calls), a.clone()),
            spawn(Arc::clone(&cache), Arc::clone(&calls), a.clone()),
        );
        let first = first.expect("join").expect("first");
        let second = second.expect("join").expect("second");

        assert_eq!(calls.load(Ordering::SeqCst), 1, "one build for two callers");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn a_failed_build_is_not_cached_and_can_retry() {
        let cache = LayerIndexCache::new(config(1024));
        let a = digest(1);

        let err = cache
            .get_or_build(&a, || Err(ApiError::internal("boom")))
            .await;
        assert!(err.is_err());
        assert!(cache.get(&a).is_none());

        let ok = cache
            .get_or_build(&a, || Ok(index(&["etc"])))
            .await
            .expect("retry succeeds");
        assert_eq!(ok.len(), 1);
        assert!(cache.get(&a).is_some());
    }

    #[tokio::test]
    async fn a_cancelled_build_does_not_leak_its_inflight_entry() {
        let cache = Arc::new(LayerIndexCache::new(config(1024)));
        let a = digest(1);

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let task = tokio::spawn({
            let cache = Arc::clone(&cache);
            let a = a.clone();
            async move {
                cache
                    .get_or_build(&a, move || {
                        let _ = started_tx.send(());
                        let _ = release_rx.recv();
                        Ok(index(&["etc"]))
                    })
                    .await
            }
        });

        started_rx.await.expect("build started");
        assert_eq!(cache.inner.lock().unwrap().inflight.len(), 1);

        // Simulate the client going away: the request future, and with it the
        // lease, is dropped while the blocking scan is still running.
        task.abort();
        let _ = task.await;
        let leaked = cache.inner.lock().unwrap().inflight.len();
        release_tx.send(()).expect("release build");

        assert_eq!(
            leaked, 0,
            "a cancelled build must not leave an in-flight entry"
        );
    }

    #[tokio::test]
    async fn a_cancelled_composed_build_does_not_leak_its_inflight_entry() {
        let generation = LayerIndexCache::new(config(1024)).generation();
        let cache = Arc::new(ComposedLayerCache::new(config(1024), generation));
        let key: ComposedKey = (digest(1), 0);

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let task = tokio::spawn({
            let cache = Arc::clone(&cache);
            let key = key.clone();
            async move {
                cache
                    .get_or_build(&key, move || {
                        let _ = started_tx.send(());
                        let _ = release_rx.recv();
                        Ok(Arc::new(ComposedLayer::empty()))
                    })
                    .await
            }
        });

        started_rx.await.expect("build started");
        assert_eq!(cache.inner.lock().unwrap().inflight.len(), 1);

        task.abort();
        let _ = task.await;
        let leaked = cache.inner.lock().unwrap().inflight.len();
        release_tx.send(()).expect("release build");

        assert_eq!(
            leaked, 0,
            "a cancelled composed build must not leave an in-flight entry"
        );
    }

    fn changes(paths: &[&str]) -> Arc<Changes> {
        let changes = paths
            .iter()
            .map(|path| ((*path).to_string(), Change::New))
            .collect();
        Arc::new(Changes { changes })
    }

    #[test]
    fn changes_cache_skips_an_entry_larger_than_the_budget() {
        let generation = LayerIndexCache::new(config(1024)).generation();
        let cache = ChangesCache::new(config(100), generation);
        cache.insert((digest(1), 0), changes(&["a"]));
        assert_eq!(cache.len(), 1, "a small change map is cached");

        // A 40-byte path weighs 40 + 64 = 104, over the 100-byte budget.
        let heavy = "x".repeat(40);
        cache.insert((digest(2), 0), changes(&[heavy.as_str()]));
        assert_eq!(cache.len(), 1, "an oversized change map is not cached");
    }

    #[test]
    fn changes_cache_expired_entry_is_treated_as_a_miss() {
        let generation = LayerIndexCache::new(config(1024)).generation();
        let cache = ChangesCache::new(
            LayerCacheConfig {
                max_bytes: 1024,
                ttl: Duration::from_millis(10),
            },
            generation,
        );
        let key: ComposedKey = (digest(1), 0);
        cache.insert(key.clone(), changes(&["etc"]));
        assert!(cache.get(&key).is_some());

        std::thread::sleep(Duration::from_millis(20));
        assert!(cache.get(&key).is_none(), "TTL elapsed");
        assert_eq!(cache.len(), 0, "expired entry is reclaimed");
    }

    #[test]
    fn changes_cache_stale_generation_is_treated_as_a_miss() {
        let layer_cache = LayerIndexCache::new(config(1024));
        let cache = ChangesCache::new(config(1024), layer_cache.generation());
        let key: ComposedKey = (digest(1), 0);
        cache.insert(key.clone(), changes(&["etc"]));
        assert!(cache.get(&key).is_some());

        layer_cache.invalidate(&digest(9));
        assert!(
            cache.get(&key).is_none(),
            "a generation advance invalidates cached classifications"
        );
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn composed_cache_sweep_stale_generation_reclaims_entries() {
        let layer_cache = LayerIndexCache::new(config(1024));
        let cache = ComposedLayerCache::new(config(1024), layer_cache.generation());
        let key: ComposedKey = (digest(1), 0);
        cache.insert(key.clone(), Arc::new(ComposedLayer::empty()));
        cache.sweep_stale_generation();
        assert_eq!(cache.len(), 1, "a current-generation overlay is kept");

        layer_cache.invalidate(&digest(9));
        cache.sweep_stale_generation();
        assert_eq!(cache.len(), 0, "a stale-generation overlay is reclaimed");
    }

    #[test]
    fn changes_cache_sweep_stale_generation_reclaims_entries() {
        let layer_cache = LayerIndexCache::new(config(1024));
        let cache = ChangesCache::new(config(1024), layer_cache.generation());
        let key: ComposedKey = (digest(1), 0);
        cache.insert(key.clone(), changes(&["etc"]));
        cache.sweep_stale_generation();
        assert_eq!(cache.len(), 1, "a current-generation entry is kept");

        layer_cache.invalidate(&digest(9));
        cache.sweep_stale_generation();
        assert_eq!(
            cache.len(),
            0,
            "a stale-generation classification is reclaimed"
        );
    }

    fn listing_key(position: u32) -> ListingKey {
        ListingKey {
            scope: "sha256:scope".to_string(),
            position,
            path: "etc".to_string(),
            mode: "single",
        }
    }

    #[test]
    fn listing_cache_evicts_the_least_recently_used_over_budget() {
        let key = listing_key(0);
        let entry_weight = |body: &[u8]| {
            body.len()
                + key.scope.len()
                + key.path.len()
                + key.mode.len()
                + LISTING_KEY_OVERHEAD_BYTES
        };
        let body = b"body";
        let budget = entry_weight(body) * 2;
        let cache = ListingCache::new(
            config(budget),
            LayerIndexCache::new(config(1024)).generation(),
        );
        let a = listing_key(0);
        let b = listing_key(1);
        let c = listing_key(2);
        cache.insert(a.clone(), Bytes::from_static(body));
        cache.insert(b.clone(), Bytes::from_static(body));
        assert_eq!(cache.len(), 2, "two listings fit inside the budget");
        assert_eq!(cache.weight(), budget);

        // Touching `a` makes `b` the least-recently-used victim.
        assert!(cache.get(&a).is_some());
        cache.insert(c.clone(), Bytes::from_static(body));

        assert_eq!(cache.len(), 2);
        assert!(cache.get(&b).is_none(), "b was evicted");
        assert!(cache.get(&a).is_some(), "a was recently used");
        assert!(cache.get(&c).is_some(), "c was just inserted");
    }

    #[test]
    fn listing_cache_expired_entry_is_treated_as_a_miss() {
        let cache = ListingCache::new(
            LayerCacheConfig {
                max_bytes: 4096,
                ttl: Duration::from_millis(10),
            },
            LayerIndexCache::new(config(1024)).generation(),
        );
        let key = listing_key(0);
        cache.insert(key.clone(), Bytes::from_static(b"body"));
        assert!(cache.get(&key).is_some());

        std::thread::sleep(Duration::from_millis(20));
        assert!(cache.get(&key).is_none(), "TTL elapsed");
        assert_eq!(cache.len(), 0, "expired entry is reclaimed");
    }

    #[test]
    fn listing_cache_stale_generation_is_treated_as_a_miss() {
        let layer_cache = LayerIndexCache::new(config(4096));
        let cache = ListingCache::new(config(4096), layer_cache.generation());
        let key = listing_key(0);
        cache.insert(key.clone(), Bytes::from_static(b"body"));
        assert!(cache.get(&key).is_some());

        layer_cache.invalidate(&digest(9));
        assert!(
            cache.get(&key).is_none(),
            "a generation advance invalidates a cached listing"
        );
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn listing_cache_sweep_stale_generation_reclaims_entries() {
        let layer_cache = LayerIndexCache::new(config(1024));
        let cache = ListingCache::new(config(1024), layer_cache.generation());
        let key = listing_key(0);
        cache.insert(key.clone(), Bytes::from_static(b"body"));
        cache.sweep_stale_generation();
        assert_eq!(cache.len(), 1, "a current-generation listing is kept");

        layer_cache.invalidate(&digest(9));
        cache.sweep_stale_generation();
        assert_eq!(cache.len(), 0, "a stale-generation listing is reclaimed");
    }

    #[test]
    fn next_deadline_tracks_the_earliest_entry_and_clears() {
        let ttl = Duration::from_millis(1000);
        let cache = LayerIndexCache::new(LayerCacheConfig {
            max_bytes: 4096,
            ttl,
        });
        assert_eq!(
            cache.next_deadline(),
            None,
            "an empty cache has no deadline"
        );

        let a = digest(1);
        let b = digest(2);
        cache.insert(a.clone(), index(&["a"]));
        let first = cache.next_deadline().expect("a deadline is set");

        std::thread::sleep(Duration::from_millis(300));
        cache.insert(b.clone(), index(&["b"]));
        assert_eq!(
            cache.next_deadline(),
            Some(first),
            "the oldest expiry remains the deadline"
        );

        std::thread::sleep(Duration::from_millis(800));
        cache.sweep_expired();
        assert!(cache.get(&b).is_some(), "b is still fresh");
        let later = cache.next_deadline().expect("b still has a deadline");
        assert!(later > first, "the deadline advanced to b's expiry");

        let remaining = later.saturating_duration_since(Instant::now());
        std::thread::sleep(remaining + Duration::from_millis(20));
        cache.sweep_expired();
        assert_eq!(cache.next_deadline(), None, "nothing left to expire");
    }
}
