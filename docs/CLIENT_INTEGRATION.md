# Client integration

For developers building a JS/web client against `pankosmia_docker`.
Covers authentication, headers, the dual-shape watch endpoint,
saving with the GitHub edit flow, conflict resolution, the
optional admin panel, and the audio upload path.

---

## 1. Mental model

The translator's experience:

> I open the app, click "Sign in with GitHub" once, and from then
> on I edit text and click Save. The app sometimes tells me "your
> edit is awaiting review" or, less often, "this passage was
> updated by someone else, please review the changes." I never go
> to github.com.

Behind the scenes the server is doing fork management, git push,
PR creation, and webhook handling — none of which the user needs
to know about. The client's job is to surface only what the
translator must see:

1. Sign-in button.
2. The text being edited.
3. Save (with a clear post-save status: published / awaiting review).
4. A conflict-resolution dialog when needed.
5. An optional "your pending edits" page.

Plus, for language admins specifically:

6. A pending-PRs panel with approve / reject controls.

---

## 2. Sign in

### 2.1 The "Sign in with GitHub" button

Standard OAuth Authorization Code flow. The server orchestrates.

```js
function signIn() {
  window.location.href =
    `${API_BASE}/auth/start?redirect=${encodeURIComponent(location.pathname)}`;
}
```

The server:

1. Generates a CSRF state token.
2. Redirects the browser to GitHub's OAuth authorize URL.
3. GitHub presents the consent screen.
4. On approval, GitHub redirects to the server's
   `/auth/callback` with `?code=...&state=...`.
5. Server validates state, exchanges code for token, fetches the
   user profile, persists the encrypted token, sets the session
   cookie, and redirects to the original page.

### 2.2 What the user sees

A familiar consent screen on github.com. It says:

> Pankosmia by **&lt;your-org&gt;** would like permission to:
>   - Access public repositories on your behalf.
>   - Read your primary email address.

If the user already has a GitHub account: ~5 seconds. If they
don't, GitHub walks them through signup, then back. ~3 minutes
one-time.

### 2.3 After sign-in

The client checks `GET /me`:

```js
const me = await fetch(`${API_BASE}/me`, { credentials: 'include' })
  .then(r => r.ok ? r.json() : null);
```

`me` is `{ github_user_id, login, name, email, avatar_url }`.
Session cookies are HttpOnly and Secure; client JS never sees the
GitHub OAuth token.

### 2.4 Sign out

```js
await fetch(`${API_BASE}/auth/logout`, {
  method: 'POST',
  credentials: 'include',
});
```

The server clears the session cookie. The encrypted OAuth token
stays on the server and is reused on next sign-in (unless the user
revokes the OAuth app on github.com — in which case the next API
call returns 401 and the server discards the token).

---

## 3. Listing languages

```js
const summaries = await authedFetch(`${API_BASE}/languages`)
  .then(r => r.json());
// [
//   { code: "en", display_name: "English", role: "viewer", direction: "ltr" },
//   { code: "fr", display_name: "French",  role: "editor", direction: "ltr" },
//   { code: "ar", display_name: "Arabic",  role: "owner",  direction: "rtl" },
// ]
```

The server combines:
- The catalog's list of registered languages.
- The caller's collaborator role on each language repo (cached
  briefly per user).

Languages the user has no role on still appear with `role:
"viewer"` because public language repos are world-readable.

---

## 4. The `X-Language-Code` header

For every per-language request:

```js
async function authedFetch(url, options = {}) {
  const headers = new Headers(options.headers);
  const lang = currentLanguage();   // your app state
  if (lang) headers.set('X-Language-Code', lang);
  return fetch(url, { ...options, headers, credentials: 'include' });
}
```

The session cookie carries identity; `X-Language-Code` carries the
working language. If a user has roles on multiple languages, this
header determines which language the current request is about.

---

## 5. Reading content

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

Reads serve from the server's local cache of the upstream repo;
no GitHub API call on the read path. The cache stays fresh via
webhooks and the periodic fetch fairing.

---

## 6. Saving — the heart of the integration

### 6.1 The simple save

```js
async function save(repoPath, ipath, newText) {
  const url =
    `${API_BASE}/burrito/post-raw/${encodeRepoPath(repoPath)}` +
    `?ipath=${encodeURIComponent(ipath)}`;
  const resp = await authedFetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'text/plain' },
    body: newText,
  });
  return resp.json();
}
```

A successful response:

```json
{
  "is_good": true,
  "status": "awaiting_review",
  "pr_number": 42,
  "pr_url": "https://github.com/.../pull/42"
}
```

Display: **"Saved (awaiting review)"** plus an optional subdued
"view pending changes" link. Most translators don't need it; the
link is for the curious.

### 6.2 The conflict case

When the upstream `main` has moved on and the user's edit
conflicts:

```json
{
  "is_good": false,
  "status": "conflict",
  "conflicts": [
    {
      "ipath": "tn/jas/01.tsv",
      "base":   "<original-text>",
      "yours":  "<text-the-user-just-saved>",
      "theirs": "<text-now-on-upstream-main>"
    }
  ]
}
```

The client renders a 3-way merge dialog:

- Three columns: "Original", "Your version", "Latest version".
- A fourth area: "Resolved version" (editable).
- Two buttons: "Keep yours" and "Take latest" (each pre-fills the
  resolved area).
- "Cancel" (discards local changes) and "Submit resolution".

```js
async function resolveConflict(repoPath, ipath, resolvedText) {
  const url =
    `${API_BASE}/burrito/resolve-conflict/${encodeRepoPath(repoPath)}` +
    `?ipath=${encodeURIComponent(ipath)}`;
  return authedFetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ resolved_text: resolvedText }),
  }).then(r => r.json());
}
```

The server completes the rebase and the response shape matches the
simple-save success case.

### 6.3 Save state machine

| `status` | Meaning | UI |
|---|---|---|
| `published` | Edit went directly to `main` (user is admin or has direct push) | "Saved (live)" |
| `awaiting_review` | Edit landed in a PR awaiting language admin merge | "Saved (awaiting review)" |
| `conflict` | Rebase failed; user must resolve | Open conflict dialog |

`published` happens when the user IS the language admin. The
common case for translators is `awaiting_review`.

### 6.4 Optimistic UI

Apply the edit locally before the save round-trip completes. On
`awaiting_review` or `published`, keep the optimistic state. On
`conflict`, roll it back and show the dialog. On HTTP error (5xx),
roll back with an error toast.

The SSE watch endpoint (see §8) eventually reconciles your local
state with whatever's on `main`.

---

## 7. The "your pending changes" page (optional but nice)

```js
const pending = await authedFetch(`${API_BASE}/me/pending-prs`)
  .then(r => r.json());
// [
//   { pr_number: 42, language: "fr", title: "Edits by Alice",
//     opened_at: "...", file_count: 3, status: "awaiting_review",
//     url: "..." },
// ]
```

Render as a list. Each entry expands to show changed files. A
"withdraw" button:

```js
await authedFetch(`${API_BASE}/me/pending-prs/${pr_number}`, {
  method: 'DELETE',
});
```

The server closes the PR via the GitHub API.

---

## 8. SSE — the watch endpoint

Same URL as the read endpoint, dispatched by the `Accept` header:

- `Accept: text/event-stream` (sent automatically by `EventSource`)
  → SSE stream of `change` events.
- Anything else → file bytes (the read path).

```js
const url =
  `${API_BASE}/burrito/ingredient/raw/${encodeRepoPath(repoPath)}` +
  `?ipath=${encodeURIComponent(ipath)}`;
const es = new EventSource(url, { withCredentials: true });

let lastHash = null;

es.addEventListener('change', (ev) => {
  const { hash } = JSON.parse(ev.data);
  if (lastHash === null) {
    lastHash = hash;
  } else if (hash !== lastHash) {
    lastHash = hash;
    refetchAndApply();
  }
});

es.addEventListener('error', () => {
  // EventSource auto-reconnects with exponential backoff. Logging
  // only.
});

// On unmount:
es.close();
```

What triggers a `change` event:

| Trigger | What happens |
|---|---|
| Translator's PR is merged on github.com | GitHub webhook → server fetches → SSE change |
| Admin merges via in-browser admin panel | Server fetches → SSE change |
| External force-push on the language repo | Webhook → server fetches → SSE change |
| User saves their own edits | NOT a change event — their edit isn't on `main` yet |

A translator's own save doesn't trigger their own SSE event. They
see their edit because the optimistic UI showed it locally. The
SSE fires later (potentially much later) when the admin merges.
Distinguish "this content is what I just saved locally" from "this
content is what's on `main`."

---

## 9. The optional admin panel

For users with admin / maintain role on a language repo.

```js
// List PRs awaiting review for this language.
const pendingForReview = await authedFetch(
  `${API_BASE}/admin/pending-prs?language=fr`
).then(r => r.json());

// Each entry expands into a server-rendered diff:
const diff = await authedFetch(
  `${API_BASE}/admin/pr-diff?pr=42`
).then(r => r.json());

// Approve and merge.
await authedFetch(`${API_BASE}/admin/approve?pr=42`, { method: 'POST' });

// Reject with reason.
await authedFetch(`${API_BASE}/admin/reject?pr=42`, {
  method: 'POST',
  body: JSON.stringify({ reason: "needs source-language alignment" }),
  headers: { 'Content-Type': 'application/json' },
});
```

Decide whether to show the panel based on the user's role:

```js
const me = await fetch(`${API_BASE}/me`, { credentials: 'include' })
  .then(r => r.json());
// me.roles = { fr: "owner", en: "viewer" }
if (me.roles[currentLanguage] === 'owner' ||
    me.roles[currentLanguage] === 'maintain') {
  showAdminPanel();
}
```

Server-side, every `/admin/*` endpoint independently re-verifies
the role via the GitHub API; client-side gating is for UI hygiene.

---

## 10. Audio upload / download

Audio bytes never transit the server.

```js
async function uploadAudio(repoPath, ipath, audioBlob) {
  // 1. Ask the server for a presigned PUT URL.
  const meta = await authedFetch(`${API_BASE}/burrito/audio/upload-url`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ repoPath, ipath, contentType: audioBlob.type }),
  }).then(r => r.json());

  // 2. PUT directly to object storage.
  const putResp = await fetch(meta.url, {
    method: 'PUT',
    headers: meta.headers,
    body: audioBlob,
  });
  if (!putResp.ok) throw new Error(`upload failed: ${putResp.status}`);

  // 3. Finalize.
  await authedFetch(`${API_BASE}/burrito/audio/finalize`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ uploadId: meta.uploadId }),
  });
}

async function audioUrl(repoPath, ipath) {
  const { url } = await authedFetch(
    `${API_BASE}/burrito/audio/download-url` +
      `?repoPath=${encodeRepoPath(repoPath)}&ipath=${encodeURIComponent(ipath)}`
  ).then(r => r.json());
  return url;
}
```

---

## 11. Error handling

### 11.1 Status codes

| Status | Meaning | Client action |
|---|---|---|
| 200 | OK | Process |
| 401 | Session expired or token revoked | Show sign-in button |
| 403 | Authenticated but not allowed | Show "no access" UI |
| 404 | Language not registered or path missing | Show error / empty state |
| 409 | Save conflicted | Open conflict resolution |
| 429 | GitHub rate limit hit | Backoff and retry |
| 5xx | Server error | Retry with backoff; surface to user |

### 11.2 GitHub-specific error codes

```json
// 401 (token revoked):
{ "is_good": false, "code": "GITHUB_REVOKED", "reason": "github token revoked" }

// 429 (rate limit):
{ "is_good": false, "code": "GITHUB_RATE_LIMIT",
  "reason": "github rate limit hit", "retry_after": 1800 }

// 502 (transient outage):
{ "is_good": false, "code": "GITHUB_UPSTREAM",
  "reason": "github upstream error" }
```

`GITHUB_REVOKED` is the most user-affecting: the user has removed
the OAuth grant on github.com and must re-authorize.

---

## 12. Local development

### 12.1 Against a server with no auth

```bash
VITE_API_BASE=http://127.0.0.1:19119 npm run dev
```

For development, run the server with `STORAGE_BACKEND=fs` (the
default). No sign-in needed. Useful for offline / quick iteration
against a local content tree.

### 12.2 Against a hosted dev server

```bash
VITE_API_BASE=https://dev.example.com npm run dev
```

You'll need:

- A GitHub OAuth App registered with the dev server's callback
  URL (`https://dev.example.com/auth/callback`).
- The dev server configured with the OAuth client ID/secret.
- The dev server's `GITHUB_WEBHOOK_SECRET` if you're testing
  webhooks.

For trusted contributors, share access to a shared dev OAuth app.
For external contributors, instruct them to register their own
OAuth app and point a local Rust server at it.

### 12.3 CORS in dev

The hosted dev server must allow your dev origin (e.g.,
`http://localhost:5173`). Add it to the CORS fairing's allowlist
on the server side.

---

## 13. Common pitfalls

**P1: Forgetting `credentials: 'include'`** on cross-origin
fetches (or SSE). Cookies don't flow without it.

**P2: Calling `signIn` in a popup window.** OAuth redirects don't
play well with popups in modern browsers (third-party cookie
rules). Use a top-level redirect.

**P3: Trying to handle the conflict dialog in a click-outside-
dismissible modal.** Conflicts can have unsaved edits; require an
explicit cancel/submit.

**P4: Showing GitHub-specific terms in the translator UI.** "PR",
"branch", "fork", "rebase" — keep these out. The translator only
needs "save", "awaiting review", "this passage was updated by
someone else."

**P5: Showing the user's GitHub login as their identity.** When
the server returns `me.name` (real name) and `me.login` (GitHub
handle), display the name. Login is server-internal.

**P6: Polling `/me/pending-prs` every few seconds.** Don't. Let
SSE do the work; refresh on user navigation.

**P7: Hardcoding `Accept: text/event-stream` on a regular fetch.**
That lands you on the SSE handler and the request hangs waiting
for events. Don't set `Accept` on read fetches; the default is
fine.

**P8: Forgetting to close `EventSource` on unmount.** React
components that open an SSE in `useEffect` must close it in the
cleanup function. Otherwise the server accumulates orphaned
streams.

---

## 14. Endpoint quick reference

| Endpoint | Method | Auth | What it does |
|---|---|---|---|
| `/auth/start` | GET | none | Begin OAuth flow |
| `/auth/callback` | GET | none | Finish OAuth flow |
| `/auth/logout` | POST | session | End session |
| `/me` | GET | session | User profile + per-language roles |
| `/languages` | GET | session | List registered languages with the caller's role |
| `/burrito/ingredient/raw/.../?ipath=...` | GET | session | Read content (or SSE if `Accept: text/event-stream`) |
| `/burrito/post-raw/.../?ipath=...` | POST | session, role≥editor | Save edits → fork+push+PR |
| `/burrito/resolve-conflict/.../?ipath=...` | POST | session | Submit conflict resolution |
| `/burrito/audio/upload-url` | POST | session | Issue presigned PUT URL |
| `/burrito/audio/finalize` | POST | session | Confirm upload, record metadata |
| `/burrito/audio/download-url` | GET | session | Issue presigned GET URL |
| `/me/pending-prs` | GET | session | List user's open PRs |
| `/me/pending-prs/<n>` | DELETE | session | Withdraw a PR |
| `/admin/pending-prs?language=...` | GET | session, role=owner | List open PRs for review |
| `/admin/pr-diff?pr=<n>` | GET | session, role=owner | Server-rendered diff |
| `/admin/approve?pr=<n>` | POST | session, role=owner | Merge PR |
| `/admin/reject?pr=<n>` | POST | session, role=owner | Close PR |
| `/webhook/catalog` | POST | HMAC | (server-internal) Catalog updated |
| `/webhook/language/<code>` | POST | HMAC | (server-internal) Language repo updated |

---

## 15. Backwards compatibility for existing pankosmia-web clients

`pankosmia_docker` retains URL and JSON envelope compatibility
with `pankosmia/pankosmia-web` v0.14.x clients where practical.
Concretely:

- The `/burrito/ingredient/raw/...?ipath=...` read URL is unchanged.
- The `/settings/languages`, `/navigation/bcv`, and `/version`
  shapes are unchanged.
- The `is_good` / `reason` JSON envelope on errors is preserved.
- The SSE watch endpoint is an additive feature on the same URL,
  dispatched by the `Accept: text/event-stream` header. Clients
  that don't request it keep getting bytes.

What's new and breaks the v0.14.x assumption:

- **Authentication is required on hosted deployments.** v0.14.x
  assumed an open server (single-tenant, behind localhost). Hosted
  callers must sign in with GitHub. Desktop deployments running
  with `STORAGE_BACKEND=fs` keep working without auth.
- **`X-Language-Code` header** is honored when present and
  required for multi-language users. v0.14.x clients without the
  header fall back to the configured default language.
- **Save responses** include `status`, `pr_url`, and `pr_number`
  fields when the GitHub backend is active. Clients that ignore
  unknown fields keep working; clients that want to surface PR
  status start using them.

A v0.14.x client running unmodified against a `pankosmia_docker`
hosted deployment will be able to read and view content. Saves
will fail without authentication. Add the sign-in flow and the
client is fully functional on the hosted backend.

---

## See also

- `docs/ARCHITECTURE.md` — design rationale.
- `docs/CATALOG_REPO_TEMPLATE.md` — how the catalog repo is set up.
- `docs/SECURITY.md` — auth and ACL details from the server side.
- `docs/HOSTING.md` — operator-facing integration contract.
