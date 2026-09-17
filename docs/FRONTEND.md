# Lighthouse — Frontend

The control plane is a Vite + React + TypeScript single-page app styled with
Tailwind CSS v4 using the **Primer (GitHub)** design system. It is built to
`frontend/dist` and served as static files by the Rust binary in production.

This document describes the structure, theming, API layer and conventions. It is
the reference for the next wave of pages.

---

## 1. Tooling & commands

| Concern | Choice |
|---|---|
| Bundler | Vite 8 |
| Language | TypeScript 6 (`tsc -b`, project references) |
| UI | React 19 |
| Routing | `react-router-dom` 7 |
| Styling | Tailwind CSS v4 (`@tailwindcss/vite`, CSS-first config) |
| Validation | Valibot 1 |
| Icons | `@primer/octicons-react` |
| Captcha | `pow-captcha-react` |
| Charts | `recharts` (installed for the analytics page in a later wave) |
| Lint | `oxlint` |

```bash
cd frontend
pnpm install
pnpm dev      # Vite dev server; proxies /api and /v2 to http://localhost:8080
pnpm build    # tsc -b && vite build  →  frontend/dist
pnpm lint     # oxlint
```

`vite.config.ts` registers the Tailwind plugin and proxies `/api` and `/v2` to
the backend so the httpOnly access cookie is issued on the Vite origin and sent
on every request during development.

---

## 2. Directory structure

```
frontend/
├── index.html              Theme-resolution bootstrap (no FOUC)
├── vite.config.ts          Tailwind plugin + /api & /v2 dev proxy
├── src/
│   ├── main.tsx            Providers + root render
│   ├── App.tsx             Route table
│   ├── index.css           Primer tokens + Tailwind @theme mapping
│   ├── api/
│   │   ├── schemas.ts      Valibot DTOs for docs/API.md & docs/AUTH.md
│   │   ├── client.ts       Typed fetch wrapper, refresh-on-401, ApiError
│   │   └── endpoints.ts    One typed binding per endpoint
│   ├── lib/
│   │   ├── theme-context.ts / theme.tsx    Theme context + provider
│   │   ├── auth-context.ts / auth.tsx      Auth context + provider
│   │   ├── paths.ts        URL/route encoding + nested-repo route parsing
│   │   ├── format.ts       bytes / time / digest formatters
│   │   ├── forms.ts        Valibot → per-field errors
│   │   ├── auth-errors.ts  Server error code → human sentence
│   │   ├── useAsync.ts     Async loader with loading/error/reload
│   │   ├── ui.ts           Button/input/label class builders
│   │   └── cx.ts           Class-name joiner
│   ├── components/
│   │   ├── primitives/     Box, Button, Label, Table, Timeline, …
│   │   ├── AppShell.tsx    Header + main + footer layout route
│   │   ├── Header.tsx      Top navigation, search, avatar menu
│   │   ├── json-viewer/    Recursive collapsible JSON viewer
│   │   ├── CaptchaField.tsx
│   │   ├── ActivityTimeline.tsx
│   │   └── …
│   └── pages/              One component per route
└── public/
```

Components are colocated by role: generic primitives under
`components/primitives/`, feature widgets directly under `components/`, and
route components under `pages/`.

---

## 3. Theming

### Tokens

All colours are CSS custom properties prefixed `--lh-` and defined twice — on
`:root` (light) and `.dark` (dark) — in `src/index.css`. They are mapped into
Tailwind's theme with `@theme inline` so utilities such as `bg-canvas-default`,
`text-muted`, `border-border`, `text-success` and `bg-danger-subtle` follow the
active palette. Because `inline` keeps the utility pointing at the variable
(not a baked-in value), the `.dark` override works without `dark:` variants.

| Semantic token | Light | Dark |
|---|---|---|
| `canvas-default` | `#ffffff` | `#0d1117` |
| `canvas-subtle` | `#f6f8fa` | `#161b22` |
| `canvas-overlay` | `#ffffff` | `#161b22` |
| `foreground` | `#1f2328` | `#e6edf3` |
| `muted` | `#636c76` | `#848d97` |
| `border` | `#d0d7de` | `#30363d` |
| `accent` | `#0969da` | `#2f81f7` |
| `success` | `#1a7f37` | `#3fb950` |
| `attention` | `#9a6700` | `#d29922` |
| `danger` | `#d1242f` | `#f85149` |

Supporting tokens (`canvas-inset`, `border-muted`, `*-emphasis`, `*-subtle`,
`neutral-subtle`) are derived from the same Primer ramps. Typography uses the
Primer system stack (14px/1.5 body, `ui-monospace` for code); radii are 6px
(`rounded-md`) and 10px (`rounded-lg`); the soft shadow is `shadow-primer`
(`0 1px 0 rgba(27,31,35,0.04)`) and menus use `shadow-medium`. Focus is a 2px
accent outline applied globally on `:focus-visible`.

Reusable primitives live in `components/primitives/`: `Box`, `Button`
(default/primary/danger, plus `LinkButton`/`AnchorButton`), `Label`, `Table`,
`Timeline`, `CounterLabel`, `Avatar`, `TextInput`, `Flash`, `LoadingState`,
`EmptyState`, `ErrorState`.

### Preference resolution

`index.html` contains a tiny inline script that resolves the theme **before
first paint**, so there is no flash of the wrong palette. `src/lib/theme.tsx`
persists the choice to `localStorage` under `lighthouse.theme`, toggles
`class="dark"` on `<html>` and sets `color-scheme`.

Resolution order:

1. Stored `lighthouse.theme` (`light` / `dark`).
2. On first visit only, `prefers-color-scheme`.
3. The server-side `theme` from `GET /api/auth/me` once the session loads (the
   `AuthProvider` calls `setTheme(user.theme)`), which wins over 1–2.

Light is the default.

---

## 4. API layer

### `src/api/schemas.ts`

A Valibot schema per DTO in `docs/API.md` and `docs/AUTH.md`, with TypeScript
types derived via `v.InferOutput`. This includes every user/social, namespace,
repository, tag, platform, layer, permission, service-account, analytics,
activity, dashboard, pagination and auth DTO, plus request payloads and
form schemas. The page envelope is generated by `pageSchema(itemSchema)`.

### `src/api/client.ts`

`api.get/post/patch/delete(path, schema, body?)` plus `postVoid`, `deleteVoid`
and `raw` for non-JSON payloads.

- Every request sends `credentials: 'include'`. **The access token is an
  httpOnly cookie and is never read or stored in JS.**
- Every JSON response is parsed with `v.safeParse`; a shape mismatch throws an
  `ApiError` with code `invalid_response`.
- Failures are normalised from the `{ "error": { code, message } }` envelope
  into an `ApiError` carrying `status`, `code` and `message`.
- On `401`, a **single-flight** refresh (`refreshSession`) rotates the refresh
  token using the value in `localStorage` (`lighthouse.refresh_token`) and
  retries the original request once. Auth endpoints pass `retry: false`.

### `src/api/endpoints.ts`

One typed function per documented endpoint, grouped by section. Path builders in
`src/lib/paths.ts` encode **each** segment individually so nested repository
names (`alice/more/complicated/app`) and digests (`sha256:…`) survive intact.

---

## 5. Routing & nested repository paths

`src/App.tsx` mounts a single `AppShell` layout route. Routes:

| Path | Page |
|---|---|
| `/`, `/explore` | Explore / landing |
| `/login`, `/register` | Auth |
| `/verify-email`, `/forgot-password`, `/reset-password` | Auth flows |
| `/dashboard` | Dashboard (auth-gated by `RequireAuth`) |
| `/:namespace` | Namespace repositories |
| `/:namespace/*` | Repository, tag or layer (dispatched) |

Repository names are variable-length, so a single `/:namespace/*` splat is
parsed by `parseRepositoryRoute` into one of three views: repository, `…/tags/:tag`
or `…/layers/:digest`. The last `tags`/`layers` segment before a single trailing
segment acts as the delimiter. A repository literally named `tags` at a deeper
path is an accepted ambiguity.

Static routes outrank `:namespace`, and the reserved first segments from
`docs/API.md` (`login`, `explore`, `dashboard`, …) never collide with a
namespace.

---

## 6. Conventions

- **No `any`, no `@ts-ignore`, no `as` casts.** Types come from Valibot
  inference; value parsers use `v.safeParse` and branch on `result.success`.
- **No non-null assertions on parsed data.** Guard with `if (!x) return …`.
- **Every async state is rendered explicitly** — `LoadingState`, `EmptyState`
  or `ErrorState`; there are no blank screens. Data loading goes through
  `useAsync(loader, key)`, which aborts in-flight requests and keeps the key in
  its internal state so loading is derived rather than set in an effect.
- **Responsive to ~768px**: tables live in horizontal-scroll containers, grids
  collapse, and the header search moves to its own row on small screens.
- **Accessibility**: inputs are labelled with hint/error wiring
  (`aria-describedby`, `aria-invalid`), interactive widgets use `aria-expanded` /
  `role="tablist"` / `role="menu"`, headings are semantic, and the shell has a
  skip-to-content link.

---

## 7. What this wave ships

Pages: Explore/landing, Login, Register, Verify e-mail, Forgot password, Reset
password, Dashboard (repositories + activity timeline + counters), Namespace
repositories, Repository detail (header, pull snippet, tag table with sizes /
platform badges / pull counts / delete), Tag detail (Manifest / Config / Layers
tabs with the recursive JSON viewer + layer table), Layer browser (breadcrumbs,
directory listing, inline text preview, binary download, large-layer notice) and
a 404 page.

Auth uses `pow-captcha-react`; `GET /api/auth/captcha` decides whether the
widget is rendered (a `204` means captcha is disabled and nothing is shown).

## 8. Deferred to the next wave (and contract notes)

- **Analytics page** — `recharts` is installed but unused; the
  `/analytics/overview` binding is ready.
- **Profile pages / follows / heatmap**, **settings** (profile, theme, password,
  sessions, login history), **service accounts**, **permission management**,
  **tags-by-size**, **namespace/workspace management** and **repository
  settings** — schemas and endpoint bindings exist; the UI is not built.
- **Pull statistics UI** — `docs/API.md` describes `GET …/pulls?days=` but not
  its response body, so it is bound as opaque JSON and rendered nowhere yet.
- **Global repository search** — there is no cross-namespace repository search
  endpoint, so Explore lists public namespaces and then fetches repositories for
  the first eight public ones; a namespace-scoped report notes this.
- **Captcha transport** — `docs/AUTH.md` mounts `pow-captcha-axum` under
  `/api/auth/captcha` (`/challenge`, `/redeem`, …). `src/routes.rs` currently
  only implements `GET /api/auth/captcha`; the React widget targets the
  documented `/api/auth/captcha/` prefix, so enabling captcha requires mounting
  `captcha::service()` server-side. With `CAPTCHA_ENABLED=false` the `204`
  short-circuit skips the widget entirely.
- **DTO shape assumptions** where the contract is terse: `RepositorySummary.path`
  is treated as the fully-qualified path (`namespace/...`) with
  `repositoryRelativePath` tolerating a relative value; `UserProfile.namespace`
  is treated as a namespace name; `/namespaces`, `/namespaces/{name}/permissions`
  and `/users/search` follow the paginated-envelope convention.
- **Avatars** — the API returns a resolved `avatar_url` when one exists and the
  frontend renders it directly. When it is absent the profile page derives a
  Libravatar URL from the signed-in account's e-mail SHA-256 (base URL from
  `VITE_LIBRAVATAR_BASE_URL`, default `https://seccdn.libravatar.org`) and
  otherwise falls back to initials.

---

## 9. Wave 3 pages

Built on the frozen foundation. Routes are registered in `src/App.tsx`, the top
navigation and account menu gained entries, and new widgets live under
`src/components/`.

| Route | Page | Notes |
|---|---|---|
| `/analytics` | `AnalyticsPage` | `recharts` disk usage, largest tags, pulls over time and top repositories; headline counters; namespace filter |
| `/tags` | `TagCleanupPage` | sortable total/unique size, shared-storage bar, multi-select batch delete |
| `/users/:username` | `ProfilePage` | follow/unfollow, paginated followers/following, public repositories, yearly contribution heatmap |
| `/settings` | `SettingsPage` | profile, profile picture (Libravatar), sessions, login history, password, security (2FA), service accounts, app passwords |
| `/new` | `WorkspaceCreatePage` | live reserved/taken workspace-name validation |
| `/namespaces/:name/settings` | `NamespaceSettingsPage` | general settings, members, delegations |
| `/repositories/:namespace/*` | `RepositorySettingsRouteView` | repository `…/settings`: description/visibility, image delegations, delete image |

Service accounts (`ServiceAccountsPanel`) and app passwords
(`AppPasswordsPanel`) are tabs on `/settings`, not standalone routes; the active
tab is deep-linked through the `tab` query parameter (for example
`/settings?tab=service-accounts`).

Supporting components: `AnalyticsCharts`, `Heatmap`, `DelegationsPanel`,
`UserAutocomplete`, `MembersPanel`, `ServiceAccountCard`, `TagSizeTable`,
`ConfirmAction`, `FormFields`, `Tabs`, `OneTimeToken`, `ProfileHeader`,
`ProfileSettingsForm`, `AvatarSettingsPanel`, `SessionsPanel`,
`LoginHistoryPanel`, `PasswordSettingsForm`, `FollowPanel`, `UserRepositories`,
`NamespaceGeneralPanel`, `RepositoryGeneralPanel`, `LibravatarAvatar`.

### Contract notes

- `src/api/endpoints.ts` and `src/api/schemas.ts` receive **append-only**
  additions: tolerant list bindings that accept either the documented page
  envelope or the bare array the shipped backend returns (`/users/search`,
  namespace members, namespace/repository permissions, service accounts), the
  session/login-history bindings, the single-grant service-account binding, and
  the wave-3 form schemas. No frozen binding is modified.
- `recharts` colours are read from the `--lh-*` tokens at runtime through a
  `MutationObserver` on the root class, so light and dark render without any
  hard-coded hex.
- Long repository names use a splat route (`/repositories/:namespace/*`); every
  segment is encoded individually with the shared path helpers.

