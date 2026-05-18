# Architecture decisions

Why the codebase is shaped the way it is. See `ARCHITECTURE.md`
for the design itself; this doc records the reasoning.

---

## D1. Operational concerns live in `pankosmia_docker`, not in a middleware service.

Authentication, bulk ops, rate limiting, audio reference validation,
per-user state — all live in the single Rust binary. No separate
middleware container.

**Why:** single deployable, single auth surface, single source of
truth. A middleware layer doubles operational overhead and forks
the API surface. Client wrappers stay thin.

**Exception:** audio bytes are externalised entirely (D2).

---

## D2. Audio bytes live externally (Internet Archive), not in the server.

Burritos store only small reference JSON files (~200 bytes). Audio
bytes live on Internet Archive or another CC-friendly host. The
server validates references at write time but never touches audio
bytes.

**Why:** GitHub repos handle large binaries poorly. A 50-story OBS
burrito with audio would be ~3 GB — unusable with git clone. IA is
free, durable, CC-enforcing, and discoverable. Zero audio
infrastructure on the server.

**Trade-off:** contributors need an IA account for the primary
upload flow. Acceptable for translation work.

See `impl/AUDIO_STRATEGY.md` for the implementation.

---

## D3. Per-user UI state is persisted server-side in SQLite.

BCV cursor, typography, and language selections are stored in SQLite
(`PANKOSMIA_SQLITE_PATH`). The `/navigation/bcv`, `/settings/*`
endpoints are real — reads return stored values, writes persist.

**Why:** server-side persistence ensures state survives across
browsers and devices. SQLite is lightweight and already used for
user language management.

**Note:** the legacy compatibility endpoints use a shared
`COMPAT_USER` identity. Real per-user state management goes through
`/user-languages/` with `AuthUser`. Legacy endpoints will gain
proper per-user auth when needed.

---

## D4. The PR review workflow is a feature, not a workaround.

Every save creates a commit on a per-user working branch and opens
a PR. This is surfaced to translators as "Saved — awaiting review"
— never as Git terminology.

**Why:** language admins need a review gate. The PR model gives them
one without custom infrastructure. Translators never see "branch",
"PR", or "fork" in the UI.

Save responses include `pr_url` and `pr_number` for admin/diagnostic
use.

---

## D5. The catalog is the source of truth for registered languages.

A language must be in the catalog before the server will serve it.
The catalog is a GitHub org whose repos are tagged
`pankosmia-language`. The server discovers them at startup and
refreshes via webhook + 15-minute poll.

**Why:** text in a public GitHub org with a normal PR workflow for
additions. Human vetting before admission. No server-side DB
needed for registration.

---

## D6. Webhooks are optional; polling is the safety net.

Webhooks deliver sub-second change propagation but aren't required.
A 15-minute periodic fetch catches anything missed.

**Why:** lower onboarding friction — language admins don't need to
wire up webhooks before their repo works. The 15-minute lag is
acceptable for translation review workflows.

---

## Open questions (deferred)

- **Conflict resolution UI** — concurrent edits on the same file
  surface as 502. A proper resolve-conflict endpoint + client UI
  is designed but not built.
- **Server-side IA credential provisioning** — mint short-lived IA
  upload credentials from a deployment-owned account, eliminating
  per-user IA signups.
- **Persistent read cache** — reduce GitHub API load at higher
  concurrency. Add when metrics justify.
