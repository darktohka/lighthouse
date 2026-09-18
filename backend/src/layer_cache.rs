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

use crate::error::{ApiError, ApiResult};
use crate::oci::digest::Digest;

/// Estimated fixed cost of one cached entry: the map node plus the inline
/// fields of [`CachedEntry`]. Path and symlink bytes are charged on top.
const ENTRY_OVERHEAD_BYTES: usize = 64;

/// Estimated fixed cost of one cached directory-size entry: the map node plus
/// the `i64` total. Directory path bytes are charged on top.
const DIR_SIZE_OVERHEAD_BYTES: usize = 40;

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

/// The merged, whiteout-resolved path map of one layer, plus the recursive
/// byte total of every directory in it. Immutable once built and shared
/// between requests, so it only needs to be cheap to read.
#[derive(Debug, Default)]
pub struct LayerIndex {
    entries: HashMap<String, CachedEntry>,
    /// Recursive uncompressed size per directory path, precomputed at build
    /// time. The root is omitted, and a path absent here is not a directory.
    dir_sizes: HashMap<String, i64>,
    /// Subtree roots this layer removes: a `.wh.<name>` target path and the
    /// directory made opaque by `.wh..wh..opq`. Removal covers the whole
    /// subtree, so no entry under a root survives in a later overlay even when
    /// its intermediate directories never appeared as explicit tar headers.
    deletes: HashSet<String>,
    weight: usize,
}

impl LayerIndex {
    /// Wraps an already-merged path map with no deletions, computing the
    /// recursive directory totals and the weight once.
    pub fn from_entries(entries: HashMap<String, CachedEntry>) -> Self {
        Self::with_deletes(entries, HashSet::new())
    }

    /// Wraps an already-merged path map together with the subtree roots this
    /// layer removes, computing the recursive directory totals and the weight
    /// once.
    pub fn with_deletes(entries: HashMap<String, CachedEntry>, deletes: HashSet<String>) -> Self {
        let dir_sizes = directory_sizes(&entries);
        let entry_weight = entries
            .iter()
            .map(|(path, entry)| path.len() + entry.weight())
            .sum::<usize>();
        let dir_weight = dir_sizes
            .keys()
            .map(|path| path.len() + DIR_SIZE_OVERHEAD_BYTES)
            .sum::<usize>();
        let delete_weight = deletes
            .iter()
            .map(|path| path.len() + ENTRY_OVERHEAD_BYTES)
            .sum::<usize>();
        Self {
            entries,
            dir_sizes,
            deletes,
            weight: entry_weight + dir_weight + delete_weight,
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

    /// Recursive uncompressed byte total of everything below `path`; the
    /// directory entry itself contributes nothing. Returns `0` for a directory
    /// with no sized descendants and for a path that is not a directory.
    pub fn dir_size(&self, path: &str) -> i64 {
        self.dir_sizes.get(path).copied().unwrap_or(0)
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
fn ancestor_dirs(entries: &HashMap<String, CachedEntry>, dirs: &mut HashSet<String>) {
    for (path, entry) in entries {
        if entry.kind == EntryKind::Dir {
            dirs.insert(path.clone());
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

/// The cumulative overlay of a manifest's layers up to one position: the
/// whiteout-resolved path map, its recursive directory totals, and the digest
/// of the layer that last supplied each path.
#[derive(Debug, Default)]
pub struct ComposedLayer {
    entries: HashMap<String, CachedEntry>,
    dir_sizes: HashMap<String, i64>,
    /// Every directory path implied by `entries`, including empty explicit
    /// directories, so a listing can tell a present directory from a ghost.
    dirs: HashSet<String>,
    sources: HashMap<String, Digest>,
    weight: usize,
}

impl ComposedLayer {
    /// An empty overlay, the identity for [`ComposedLayer::compose`].
    pub fn empty() -> Self {
        Self::default()
    }

    /// Removes every path removed by `upper` and inserts `upper`'s entries on
    /// top, recording `upper_digest` as the source of each inserted path.
    pub fn compose(base: &ComposedLayer, upper: &LayerIndex, upper_digest: &Digest) -> Self {
        let mut entries: HashMap<String, CachedEntry> = HashMap::new();
        let mut sources: HashMap<String, Digest> = HashMap::new();
        for (path, entry) in &base.entries {
            if removed_by(path, upper.deletes()) {
                continue;
            }
            entries.insert(path.clone(), entry.clone());
            if let Some(source) = base.sources.get(path) {
                sources.insert(path.clone(), source.clone());
            }
        }
        for (path, entry) in upper.entries() {
            entries.insert(path.clone(), entry.clone());
            sources.insert(path.clone(), upper_digest.clone());
        }
        Self::from_parts(entries, sources)
    }

    fn from_parts(entries: HashMap<String, CachedEntry>, sources: HashMap<String, Digest>) -> Self {
        let dir_sizes = directory_sizes(&entries);
        let mut dirs: HashSet<String> = HashSet::new();
        ancestor_dirs(&entries, &mut dirs);
        let entry_weight = entries
            .iter()
            .map(|(path, entry)| path.len() + entry.weight())
            .sum::<usize>();
        let dir_weight = dir_sizes
            .keys()
            .map(|path| path.len() + DIR_SIZE_OVERHEAD_BYTES)
            .sum::<usize>();
        let source_weight = sources
            .keys()
            .map(|path| path.len() + ENTRY_OVERHEAD_BYTES)
            .sum::<usize>();
        let dirs_weight = dirs
            .iter()
            .map(|path| path.len() + DIR_SIZE_OVERHEAD_BYTES)
            .sum::<usize>();
        Self {
            entries,
            dir_sizes,
            dirs,
            sources,
            weight: entry_weight + dir_weight + source_weight + dirs_weight,
        }
    }

    /// The cumulative merged path map.
    pub fn entries(&self) -> &HashMap<String, CachedEntry> {
        &self.entries
    }

    /// The digest of the layer that last supplied `path`, when known.
    pub fn source(&self, path: &str) -> Option<&Digest> {
        self.sources.get(path)
    }

    /// Recursive uncompressed byte total below `path`.
    pub fn dir_size(&self, path: &str) -> i64 {
        self.dir_sizes.get(path).copied().unwrap_or(0)
    }

    /// True when `path` is a directory in this overlay, explicitly or as the
    /// prefix of a deeper entry.
    pub fn is_dir(&self, path: &str) -> bool {
        self.entries
            .get(path)
            .is_some_and(|entry| entry.kind == EntryKind::Dir)
            || self.dirs.contains(path)
    }

    /// True when this overlay holds `path` itself or any path beneath it.
    pub fn contains_path(&self, path: &str) -> bool {
        self.entries.contains_key(path) || self.dirs.contains(path)
    }

    /// Estimated heap footprint of this overlay in bytes.
    pub fn weight(&self) -> usize {
        self.weight
    }

    /// Number of merged paths.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resolves a symlink against this overlay's path map.
    pub fn resolve_link(&self, link_path: &str, target: &str) -> Option<(String, EntryKind)> {
        resolve_link_in(&self.entries, link_path, target)
    }
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
}

/// Classifies every path in `final_layer` relative to `lower_layer`. Directory
/// colour is derived from descendants, never from a synthesized directory's
/// placeholder mode.
pub fn classify(final_layer: &ComposedLayer, lower_layer: &ComposedLayer) -> Changes {
    let mut dirs: HashSet<String> = HashSet::new();
    ancestor_dirs(&final_layer.entries, &mut dirs);
    ancestor_dirs(&lower_layer.entries, &mut dirs);

    let mut lower_dirs: HashSet<String> = HashSet::new();
    ancestor_dirs(&lower_layer.entries, &mut lower_dirs);

    let mut final_desc: HashSet<String> = HashSet::new();
    for path in final_layer.entries.keys() {
        add_ancestors(path, &mut final_desc);
    }
    let mut lower_desc: HashSet<String> = HashSet::new();
    for path in lower_layer.entries.keys() {
        add_ancestors(path, &mut lower_desc);
    }

    let mut changes: HashMap<String, Change> = HashMap::new();

    // A path that is a directory in one layer and a leaf in the other is a type
    // modification; it never participates in directory rollup.
    let mut conflicts: HashSet<String> = HashSet::new();
    for (path, final_entry) in &final_layer.entries {
        if let Some(lower_entry) = lower_layer.entries.get(path) {
            if (final_entry.kind == EntryKind::Dir) != (lower_entry.kind == EntryKind::Dir) {
                changes.insert(path.clone(), Change::Modified);
                conflicts.insert(path.clone());
            }
        }
    }
    for path in &conflicts {
        dirs.remove(path);
    }

    for (path, final_entry) in &final_layer.entries {
        if conflicts.contains(path) || final_entry.kind == EntryKind::Dir {
            continue;
        }
        match lower_layer.entries.get(path) {
            None => {
                changes.insert(path.clone(), Change::New);
            }
            Some(lower_entry) => {
                if !same_entry(final_entry, lower_entry) {
                    changes.insert(path.clone(), Change::Modified);
                }
            }
        }
    }
    for (path, lower_entry) in &lower_layer.entries {
        if conflicts.contains(path) || final_layer.entries.contains_key(path) {
            continue;
        }
        if lower_entry.kind == EntryKind::Dir {
            continue;
        }
        changes.insert(path.clone(), Change::Removed);
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
        let change =
            if lower_dirs.contains(path) && !final_d && !final_layer.entries.contains_key(path) {
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

/// Folds every non-directory entry's size into each of its ancestor
/// directories, so a directory's recursive total is available without walking
/// the map again.
fn directory_sizes(entries: &HashMap<String, CachedEntry>) -> HashMap<String, i64> {
    let mut dir_sizes: HashMap<String, i64> = HashMap::new();
    for (path, entry) in entries {
        if entry.kind == EntryKind::Dir || entry.size <= 0 {
            continue;
        }
        for (index, byte) in path.bytes().enumerate() {
            if byte == b'/' {
                let total = dir_sizes.entry(path[..index].to_string()).or_insert(0);
                *total = total.saturating_add(entry.size);
            }
        }
    }
    dir_sizes
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
    /// 64 MiB: several hundred typical image layers, or a handful of layers at
    /// the 100k-entry cap, while staying well inside a small container's RAM.
    pub const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;
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

/// Concurrent, bounded, self-expiring cache of layer indices.
pub struct LayerIndexCache {
    config: LayerCacheConfig,
    inner: Mutex<Inner>,
    /// Number of expired-entry sweeps performed.
    sweeps: AtomicU64,
    /// Bumped whenever an index is invalidated, so caches derived from indices
    /// (the composed-overlay cache) can refuse entries built before the change.
    generation: Arc<AtomicU64>,
}

impl LayerIndexCache {
    pub fn new(config: LayerCacheConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(Inner::default()),
            sweeps: AtomicU64::new(0),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The shared generation counter, incremented by [`Self::invalidate`] and
    /// [`Self::clear`]. A derived cache checks it to avoid serving data built
    /// from an index that has since been dropped.
    pub fn generation(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.generation)
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

        let slot = self.build_lock(digest);
        let _guard = slot.lock().await;

        // Another caller may have populated the cache while we waited.
        if let Some(index) = self.get(digest) {
            self.finish_build(digest);
            return Ok(index);
        }

        let result = match tokio::task::spawn_blocking(build).await {
            Ok(Ok(index)) => {
                self.insert(digest.clone(), Arc::clone(&index));
                Ok(index)
            }
            Ok(Err(err)) => Err(err),
            Err(err) => {
                tracing::error!(error = %err, "layer index build panicked");
                Err(ApiError::internal("layer reader failed"))
            }
        };
        self.finish_build(digest);
        result
    }

    /// Drops the cached index for `digest`, e.g. after its blob is removed, and
    /// advances the generation so no derived overlay serves pre-invalidation
    /// data.
    pub fn invalidate(&self, digest: &Digest) {
        self.lock().remove(digest);
        self.generation.fetch_add(1, Ordering::SeqCst);
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

    fn build_lock(&self, digest: &Digest) -> Arc<tokio::sync::Mutex<()>> {
        let mut inner = self.lock();
        Arc::clone(
            inner
                .inflight
                .entry(digest.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    }

    fn finish_build(&self, digest: &Digest) {
        self.lock().inflight.remove(digest);
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

        let slot = self.build_lock(key);
        let _guard = slot.lock().await;

        if let Some(layer) = self.get(key) {
            self.finish_build(key);
            return Ok(layer);
        }

        let generation = self.current_generation();
        let result = match tokio::task::spawn_blocking(build).await {
            Ok(Ok(layer)) => {
                self.insert_with_generation(key.clone(), Arc::clone(&layer), generation);
                Ok(layer)
            }
            Ok(Err(err)) => Err(err),
            Err(err) => {
                tracing::error!(error = %err, "composed overlay build panicked");
                Err(ApiError::internal("layer reader failed"))
            }
        };
        self.finish_build(key);
        result
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

    fn build_lock(&self, key: &ComposedKey) -> Arc<tokio::sync::Mutex<()>> {
        let mut inner = self.lock();
        Arc::clone(
            inner
                .inflight
                .entry(key.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    }

    fn finish_build(&self, key: &ComposedKey) {
        self.lock().inflight.remove(key);
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
    fn directory_sizes_are_summed_recursively() {
        let entries: HashMap<String, CachedEntry> = [
            file_entry("etc/config", 5),
            file_entry("usr/bin/run", 10),
            file_entry("usr/lib/lib.so", 20),
            ("usr".to_string(), CachedEntry::directory()),
        ]
        .into_iter()
        .collect();
        let index = LayerIndex::from_entries(entries);

        assert_eq!(index.dir_size("etc"), 5);
        assert_eq!(index.dir_size("usr"), 30, "nested files roll up");
        assert_eq!(index.dir_size("usr/bin"), 10);
        assert_eq!(index.dir_size("usr/lib"), 20);
        assert_eq!(index.dir_size("var"), 0, "unknown directory");
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
}
