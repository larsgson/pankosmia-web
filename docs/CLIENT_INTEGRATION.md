# Client integration

For developers building a JS/web client against `pankosmia_docker`.
Covers the two backend modes, sign-in, the read/save/watch loop, the
admin / review panel, error handling, the endpoint surface, and the
features that are deliberately out of scope for now.

A v0.14.x `pankosmia-web` client works against this server with no
code changes when the server runs in **FS mode**. When the server
runs in **GitHub mode** (hosted, multi-tenant), the client needs to
sign in and to set one extra header on per-language requests — see
§1 and §4.

---

## 1. Compatibility with pankosmia-web

`pankosmia_docker` is a fork of [pankosmia/pankosmia-web](https://github.com/pankosmia/pankosmia-web)
with a hosted, GitHub-backed mode added. The single-tenant
filesystem-backed mode is preserved verbatim, and that's the source
of compatibility:

- **All read endpoints** (`/burrito/ingredient/raw/...`, `/version`,
  `/settings/...`, `/navigation/bcv`, `/i18n/...`, etc.) have the
  same URLs, methods, query strings, and JSON shapes as in
  pankosmia-web.
- **All save endpoints** keep their URLs and request/response
  shapes. The server returns the same `{"is_good": true, "reason":
  "ok"}` envelope on success in FS mode. In GitHub mode the
  envelope grows additional fields (`pr_number`, `pr_url`, `branch`,
  `status`); clients that ignore unknown fields keep working.
- **SSE on the read URL** (`Accept: text/event-stream`) is the same
  shape as before.

What changes when the deployment runs **GitHub mode**:

- The client must sign in (§3) before save / admin requests succeed.
- Per-language requests must include the `X-Language-Code` header
  (§4). Read endpoints don't require it (the catalog routes by
  `<repo_path>` for legacy reads), but save-style endpoints do.
- A few endpoints are intentionally not implemented in GitHub mode
  yet — see §13 for the 501 list.
- Per-user app state (BCV cursor, typography) is **not persisted
  server-side**. The endpoints exist for compatibility but return
  defaults / silently accept writes; clients should keep this in
  `localStorage`.

A v0.14.x client running unmodified against a GitHub-mode server
will be able to view content. For editing, layer the sign-in flow on
top and add the language header.

---

## 2. The two backend modes

| | `STORAGE_BACKEND=fs` | `STORAGE_BACKEND=github` |
|---|---|---|
| Source of truth | Local workspace tree | Per-language GitHub repos |
| Auth | None | GitHub-App user-authorisation (session cookie) |
| Per-language header | Ignored | Required for saves |
| Save mechanism | `fs::write` to the working tree | Branch + commit + PR via the App's installation token |
| Save response | `{is_good: true, reason: "ok"}` | adds `status`, `branch`, `pr_url`, `pr_number` |
| Multi-file ops | Implemented | 501 (deferred — see §13) |
| Audio path | Not built | Not built |

Endpoint **URLs are the same** across both modes; the server picks
the backend at boot.

---

## 3. Sign in (GitHub mode only)

The server orchestrates the OAuth-shaped flow. Identity only — the
GitHub App user-authorisation flow requests **no scopes**.

### 3.1 Start

```js
function signIn() {
  window.location.href =
    `${API_BASE}/auth/start?redirect=${encodeURIComponent(location.pathname)}`;
}
```

Server:

1. Generates a CSRF state token in a HttpOnly cookie.
2. Redirects the browser to GitHub's authorize URL for the App.
3. After approval, GitHub redirects to `${API_BASE}/auth/callback?code=...&state=...`.
4. Server validates state, exchanges code for a user-to-server
   token, calls `GET /user` to capture the login, stores the token
   AES-GCM-encrypted server-side, sets the session cookie, and
   redirects to the `redirect` path.

The browser never sees the GitHub user-to-server token; it only
holds the session cookie.

### 3.2 What the consent screen shows

`Authorize <Your-GitHub-App-Name>`. No permissions listed — the
App requests no user-scoped access. (Repo writes are done by the
**App's installation token**, not the user's token.)

### 3.3 After sign-in

```js
const me = await fetch(`${API_BASE}/me`, { credentials: 'include' })
  .then(r => r.ok ? r.json() : null);
// { github_user_id, login, name, email, avatar_url }
```

A `404` on `/me` (or the response's `is_good: false`) means the
session isn't valid — show the sign-in button.

### 3.4 Sign out

```js
await fetch(`${API_BASE}/auth/logout`, {
  method: 'POST',
  credentials: 'include',
});
```

Clears the session cookie. The encrypted token stays on the server
for re-use on next sign-in.

---

## 4. The `X-Language-Code` header (GitHub mode)

Every save / admin request must declare which language it targets:

```js
async function authedFetch(url, options = {}) {
  const headers = new Headers(options.headers);
  const lang = currentLanguage();   // your app state
  if (lang) headers.set('X-Language-Code', lang);
  return fetch(url, { ...options, headers, credentials: 'include' });
}
```

The session cookie carries identity; `X-Language-Code` (BCP 47
subset; e.g. `en`, `fr-CA`, `zh-Hans`) carries the working language.

Reads keep using the legacy `<repo_path>` URL segments and do not
require the header.

---

## 5. Reading content

Unchanged from pankosmia-web. GET the same URL the save uses:

```js
async function readIngredient(repoPath, ipath) {
  const url =
    `${API_BASE}/burrito/ingredient/raw/${encodeRepoPath(repoPath)}` +
    `?ipath=${encodeURIComponent(ipath)}`;
  return authedFetch(url).then(r => r.text());
}

function encodeRepoPath(parts) {
  return parts.map(p => encodeURIComponent(p)).join('/');
}
```

In GitHub mode, reads serve from the server's local clone of the
upstream repo — no GitHub API call on the read path. The clone is
refreshed by the language webhook (§7).

---

## 6. Saving

### 6.1 Text content — `POST /burrito/ingredient/raw/<repo_path>?ipath=...`

```js
async function save(repoPath, ipath, newText) {
  const url =
    `${API_BASE}/burrito/ingredient/raw/${encodeRepoPath(repoPath)}` +
    `?ipath=${encodeURIComponent(ipath)}`;
  const resp = await authedFetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ payload: newText }),
  });
  return resp.json();
}
```

Request body shape: `{ "payload": "<the new text>" }` — same as
pankosmia-web's FS endpoint.

**FS-mode success:**
```json
{ "is_good": true, "reason": "ok" }
```

**GitHub-mode success:**
```json
{
  "is_good": true,
  "status": "saved",
  "branch": "pankosmia-edit-<login>",
  "pr_url":  "https://github.com/<owner>/<repo>/pull/42",
  "pr_number": 42
}
```

Multiple saves from one user accumulate as commits on the same
working branch and PR. The reviewer "Squash and merge"s at the end
to land them as a single commit on `main`.

### 6.2 Binary content — `POST /burrito/ingredient/bytes/<repo_path>?ipath=...`

Multipart upload; same URL shape as pankosmia-web. The form field
name is `file`.

```js
async function saveBytes(repoPath, ipath, blob) {
  const fd = new FormData();
  fd.append('file', blob);
  const url =
    `${API_BASE}/burrito/ingredient/bytes/${encodeRepoPath(repoPath)}` +
    `?ipath=${encodeURIComponent(ipath)}`;
  return authedFetch(url, { method: 'POST', body: fd }).then(r => r.json());
}
```

**GitHub-mode cap:** 700 KB raw bytes (the GitHub Contents API
limits the request to ~1 MB base64-encoded; we leave headroom).
Larger uploads return 413; see §10.

### 6.3 Delete — `POST /burrito/ingredient/delete/<repo_path>?ipath=...`

- **FS mode**: soft delete — renames the file to `<file>.bak`.
- **GitHub mode**: hard delete from the working branch (the file is
  removed on the PR's diff; reviewer can `Revert and merge` if they
  change their mind).

### 6.4 Revert — `POST /burrito/ingredient/revert/<repo_path>?ipath=...`

- **FS mode**: restores `<file>.bak` over `<file>`.
- **GitHub mode**: replaces the working-branch version with
  upstream HEAD's version of the file (or deletes the file from the
  branch if it doesn't exist upstream).

### 6.5 Copy — `POST /burrito/ingredient/copy/<repo_path>?src_path=...&target_path=...&delete_src=true|false`

Same URL/params as pankosmia-web. In GitHub mode this becomes two
Contents-API calls (read source from the working branch, write to
target; optionally delete source).

### 6.6 What about update_ingredients / no_bak?

These query params on `/burrito/ingredient/raw/...` are honoured in
FS mode (same behaviour as pankosmia-web). In GitHub mode they are
silently ignored — there's no `.bak` concept in the App flow, and
metadata regeneration runs in a separate (deferred) endpoint.

---

## 7. Watching via SSE

Same URL as the read endpoint, dispatched by the `Accept` header:

- `Accept: text/event-stream` (auto-sent by `EventSource`) → SSE.
- Anything else (or no `Accept`) → file bytes.

```js
const url =
  `${API_BASE}/burrito/ingredient/raw/${encodeRepoPath(repoPath)}` +
  `?ipath=${encodeURIComponent(ipath)}`;
const es = new EventSource(url, { withCredentials: true });

let lastHash = null;
es.addEventListener('change', (ev) => {
  const { hash } = JSON.parse(ev.data);
  if (lastHash !== null && hash !== lastHash) {
    refetchAndApply();
  }
  lastHash = hash;
});

es.addEventListener('error', () => { /* EventSource auto-reconnects */ });
// On unmount:
es.close();
```

What triggers a `change` event in **GitHub mode**:

| Trigger | What happens |
|---|---|
| Language admin merges a PR | GitHub webhook → server fetches upstream → file mtimes change → SSE `change` |
| Direct push to the language repo (admin bypassed the app) | Webhook → fetch → SSE `change` |
| Translator saves their own edit | NOT a `change` event — their edit isn't on upstream's tracked branch yet |

A translator's own save doesn't fire their own SSE. They see their
edit via optimistic UI; the SSE fires later when the admin merges.

---

## 8. Admin / review panel

Visible to users with `admin` or `maintain` permission on the
language's GitHub repo. The server re-verifies permission on every
admin request via the GitHub collaborators API; client-side gating
is for UI hygiene only.

```js
// List the pankosmia-edit PRs awaiting review.
const pending = await authedFetch(
  `${API_BASE}/admin/pending-prs?language=fr`
).then(r => r.json());
// { is_good: true, pending: [{ pr_number, pr_url, title, submitter_login, created_at, ... }] }

// Inspect a PR's changed files (returns GitHub's `files` API shape,
// including the diff `patch` per file).
const files = await authedFetch(
  `${API_BASE}/admin/pr-files?language=fr&pr=42`
).then(r => r.json());
// { is_good: true, files: [{ filename, status, additions, deletions, patch, raw_url, ... }] }

// Approve + merge (defaults to squash; pass &method=merge|rebase to override).
await authedFetch(
  `${API_BASE}/admin/approve?language=fr&pr=42`,
  { method: 'POST' }
).then(r => r.json());
// { is_good: true, merge_method: "squash", merge_sha: "<sha>", approver_login: "..." }

// Reject (close without merging). Optional reason posted as a PR comment.
await authedFetch(
  `${API_BASE}/admin/reject?language=fr&pr=42` +
    `&reason=${encodeURIComponent("needs source-language alignment")}`,
  { method: 'POST' }
).then(r => r.json());
// { is_good: true, closed_by_login: "...", reason_recorded: true }
```

Admin permission is **per language**. There is no cross-language
super-admin.

---

## 9. Health and version

- `GET /version` — same shape as pankosmia-web (`pkg_version`,
  `product_*`, `product_resources`).
- `GET /health` — readiness probe. `200 {"status":"ok",...}` when
  the catalog is loaded AND the App is configured (in GitHub mode);
  `503 {"status":"degraded", "reasons":[...]}` otherwise. Use for
  reverse-proxy traffic-shifting; do not use as a user-facing
  readiness signal.

---

## 10. Error handling

### 10.1 Status codes

| Status | Meaning | Client action |
|---|---|---|
| 200 | OK | Process the body |
| 400 | Bad request (e.g. malformed `X-Language-Code` or `ipath`) | Surface the `reason` |
| 401 | Not signed in / session expired / token revoked | Show the sign-in button |
| 403 | Signed in but lacks permission (admin endpoints) | Hide the panel; surface a message |
| 404 | Language not in catalog, or ingredient not found | Show an empty state |
| 413 | Payload too large (GitHub-mode 700 KB cap on raw/bytes saves) | Surface the limit to the user |
| 429 | Per-user save rate limit exceeded (60 saves / 15 min) | Backoff per the `reason` text's `retry in <N>s` hint |
| 501 | Endpoint not yet implemented in GitHub mode (multi-file ops) | Disable the affected UI |
| 5xx / 502 | Server or GitHub-upstream error | Retry with backoff, surface to user |

### 10.2 Envelope

All errors return the same JSON shape:

```json
{ "is_good": false, "reason": "<human-readable>" }
```

No structured error codes (yet). Client code should branch on HTTP
status and, secondarily, on the `reason` string. (Stable
machine-readable codes are deliberately not promised — operationally
we may change reason strings as bugs are fixed.)

---

## 11. Local development

### 11.1 Against an FS-mode dev server

```bash
VITE_API_BASE=http://127.0.0.1:19119 npm run dev
```

No sign-in needed. Save/read/watch all work without auth. Best for
fast iteration; matches the pankosmia-web dev experience.

### 11.2 Against a GitHub-mode dev server

```bash
VITE_API_BASE=https://dev.example.com npm run dev
```

You'll need a GitHub App (see `docs/HOSTING.md` §3) configured with
the dev origin's callback URL (`/auth/callback`). One shared App
across multiple developers is fine; the App's user-to-server flow
doesn't grant the dev anything beyond their own identity.

### 11.3 CORS in dev

The hosted dev server must allow your dev origin (e.g.
`http://localhost:5173`). Add to the CORS allowlist on the server
side.

---

## 12. Common pitfalls

**P1: Forgetting `credentials: 'include'`** on cross-origin fetches
(or SSE `withCredentials: true`). Cookies don't flow without it.

**P2: Using a popup for sign-in.** Modern browsers' third-party
cookie rules break OAuth-style flows in popups. Use a top-level
redirect.

**P3: Mixing `localhost` and `127.0.0.1` in dev.** Browsers treat
them as different origins; the cookie set during `/auth/start` on
one won't be sent to `/auth/callback` on the other. Pick one and
stick to it through `PANKOSMIA_PUBLIC_ORIGIN`.

**P4: Sending `Accept: text/event-stream` on a regular fetch.** It
lands on the SSE handler and your request hangs. Don't set `Accept`
on reads; the default is fine.

**P5: Forgetting to close `EventSource` on unmount.** React
components must close it in their `useEffect` cleanup; otherwise the
server accumulates orphaned streams.

**P6: Persisting BCV / typography in pankosmia-docker storage.**
In GitHub mode those endpoints are stubs that return defaults. Keep
per-user UI state in `localStorage`.

**P7: Showing GitHub-side terms in the translator UI.** "PR",
"branch", "fork" — keep these out. The translator only needs
"save" + "this passage was updated by someone else".

**P8: Showing the user's GitHub `login` as their display name.**
Prefer `name` (real name) and fall back to `login`. The login is
server-internal identity, not display copy.

---

## 13. Endpoint quick reference

Read / watch (no auth needed in FS mode; session in GitHub mode):

| Endpoint | Method | Notes |
|---|---|---|
| `/burrito/ingredient/raw/<repo>?ipath=...` | GET | File bytes |
| `/burrito/ingredient/raw/<repo>?ipath=...` (with `Accept: text/event-stream`) | GET | SSE `change` stream |
| `/burrito/ingredient/bytes/<repo>?ipath=...` | GET | Raw bytes (binary) |
| `/burrito/ingredients/raw/<repo>?ipath=...` | GET | Multiple ingredients (dir listing) |
| `/burrito/metadata/raw/<repo>` | GET | `metadata.json` |
| `/burrito/metadata/summary/<repo>` | GET | Compact metadata |
| `/burrito/metadata/summaries` | GET | All summaries |
| `/burrito/paths/<repo>` | GET | File listing |

Write (session required + `X-Language-Code` in GitHub mode):

| Endpoint | Method | Notes |
|---|---|---|
| `/burrito/ingredient/raw/<repo>?ipath=...` | POST | JSON body `{payload: "..."}` |
| `/burrito/ingredient/bytes/<repo>?ipath=...` | POST | multipart `file` field |
| `/burrito/ingredient/delete/<repo>?ipath=...` | POST | Delete (FS soft / GitHub hard) |
| `/burrito/ingredient/revert/<repo>?ipath=...` | POST | Restore previous content |
| `/burrito/ingredient/copy/<repo>?src_path=&target_path=&delete_src=` | POST | Copy / move |

Auth (GitHub mode only):

| Endpoint | Method | Notes |
|---|---|---|
| `/auth/start?redirect=<path>` | GET | Begin sign-in |
| `/auth/callback?code=&state=` | GET | Server-side; the browser lands here after GitHub |
| `/auth/logout` | POST | Clear session |
| `/me` | GET | User profile |

Admin (session + `admin`/`maintain` permission on the upstream repo):

| Endpoint | Method | Notes |
|---|---|---|
| `/admin/pending-prs?language=<code>` | GET | List pankosmia-edit PRs |
| `/admin/pr-files?language=<code>&pr=<n>` | GET | Files + diff patches |
| `/admin/approve?language=<code>&pr=<n>&method=<squash\|merge\|rebase>` | POST | Merge |
| `/admin/reject?language=<code>&pr=<n>&reason=<text>` | POST | Close + optional comment |

System:

| Endpoint | Method | Notes |
|---|---|---|
| `/version` | GET | pkg + product version JSON |
| `/health` | GET | Readiness (200 ok / 503 degraded) |
| `/webhook/catalog` | POST | HMAC-signed; called by GitHub |
| `/webhook/language/<code>` | POST | HMAC-signed; called by GitHub |

State endpoints kept for pankosmia-web compatibility (FS-backed
behaviour preserved; GitHub-mode returns defaults / accepts
silently):

| Endpoint | Method | Notes |
|---|---|---|
| `/settings/languages` | GET, POST | Languages selected |
| `/settings/typography` | GET, POST | Font set, size, direction |
| `/navigation/bcv` | GET, POST | Book/chapter/verse cursor |
| `/app-state/current-project` | GET, POST | Current `source/org/project` |
| `/i18n/...` | various | UI string negotiation |

---

## 14. Not yet implemented (in GitHub mode)

Returns 501 with a clear `reason`:

- `POST /burrito/ingredients/delete/<repo>?ipath=...` — bulk
  directory delete. Needs Git Data API for atomic multi-file commit.
- `POST /burrito/metadata/remake-ingredients/<repo>` — regenerate
  metadata from the ingredients listing. Needs a tree-walk via the
  GitHub trees API.
- `POST /burrito/ingredient/zipped/<repo>?ipath=...` — zip upload
  → many files. Needs Git Data API.
- `POST /burrito/zipped/<repo>` — replace the entire repo from a
  zip. Same.

Not yet wired at all:

- **Conflict resolution.** A future
  `POST /burrito/resolve-conflict/<repo>?ipath=...` will accept the
  user's resolved bytes after the server returns a 409 with
  three-way diff data. Until then a conflict surfaces as a 502 from
  the underlying GitHub PUT.
- **Audio off the request path.** Future `/burrito/audio/upload-url`
  / `/burrito/audio/finalize` / `/burrito/audio/download-url` will
  hand the browser short-lived presigned PUT/GET URLs against an
  object-storage backend. Today, audio uploads go through the bytes
  endpoint (capped at 700 KB, see §6.2) — fine for small clips, not
  for production audio.
- **`/me/pending-prs`** (translator-facing list of one's own open
  edits). Coming alongside the trusted-contributors mitigation
  documented in `docs/SECURITY.md` §4.

When these land, the URLs above will be the ones to call.

---

## See also

- `docs/ARCHITECTURE.md` — design rationale and trust topology.
- `docs/CATALOG_REPO_TEMPLATE.md` — setting up the language catalog.
- `docs/SECURITY.md` — auth, ACLs, and abuse-mitigation details.
- `docs/HOSTING.md` — operator setup for the GitHub-App backend.
