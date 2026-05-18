# User language management

This document describes the server-side user language system and
what clients need to do to integrate with it.

---

## Overview

Each authenticated user can **claim** one or more languages from
the catalog (the set of language repos discovered from the
`pankosmia-langs` GitHub org). One of the claimed languages is
the **current language** — this determines which GitHub repo the
server reads from and writes to on behalf of the user.

Content-editing clients (munchers) do not need to know about
languages or repos. They call the same `/burrito/*` endpoints as
before. The server resolves the target repo from the user's
current language automatically.

**The dashboard (or a similar host client) is responsible for
providing a UI that lets users browse available languages, claim
one, and switch between claimed languages.**

---

## Endpoints

All endpoints are mounted under `/user-languages` and require an
authenticated session (GitHub OAuth login).

### Discovery

| Method | Path | Response |
|--------|------|----------|
| GET | `/user-languages/available-languages` | JSON array of all catalog languages |

Response shape:
```json
[
  {
    "code": "yua",
    "display_name": "Yucatec Maya",
    "direction": "ltr",
    "script": "Latn"
  }
]
```

### Claiming and releasing

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/user-languages/claim-language/<code>` | Claim a language for editing |
| POST | `/user-languages/release-language/<code>` | Release a claimed language |
| GET | `/user-languages/my-languages` | List the user's claimed language codes |

`my-languages` returns a JSON array of language code strings:
```json
["yua", "fr"]
```

**Claim rules:**
- The language must exist in the catalog.
- A user can claim up to `PANKOSMIA_MAX_USER_LANGUAGES` languages
  (default: 5).
- Claiming a language that is already claimed is a no-op (200).
- When claiming the first language, it is automatically set as
  the current language.

**Release rules:**
- If the released language was the current language, the server
  switches to the first remaining claimed language (or clears
  current if none remain).

### Switching

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/user-languages/current-language` | Get the active language |
| POST | `/user-languages/current-language/<code>` | Set the active language |

`current-language` GET returns a JSON string or null:
```json
"yua"
```

**Switch rules:**
- The language must be in the user's claimed list. Switching to
  an unclaimed language returns 403.

---

## What clients need to implement

### Dashboard (or host shell)

The dashboard must provide a language management UI. Suggested
flow:

1. **On load:** call `GET /user-languages/current-language`.
   - If non-null, the user has an active language. Show it in
     the header/status area.
   - If null, prompt the user to select a language.

2. **Language picker dialog:** call
   `GET /user-languages/available-languages` to list all
   available languages. Call `GET /user-languages/my-languages`
   to show which ones the user has already claimed.

3. **Claim:** when the user picks a new language, call
   `POST /user-languages/claim-language/<code>`. If this is
   their first language, it becomes current automatically.

4. **Switch:** when the user switches between claimed languages,
   call `POST /user-languages/current-language/<code>`.

5. **Release (optional):** provide a way to release a language
   the user no longer works on via
   `POST /user-languages/release-language/<code>`.

### Content-editing clients (munchers)

**No changes required.** The server resolves the target repo
from the user's current language. Munchers call the same
`/burrito/*` endpoints as before.

The `X-Language-Code` header is still accepted as an override
for backwards compatibility, but it is no longer required.

---

## How it connects to GitHub

Each catalog language maps to a repo in the `pankosmia-langs`
GitHub org (e.g. `yua` maps to `pankosmia-langs/yua`).

When a user edits content:

1. The server reads their current language from the session.
2. It looks up the upstream repo from the catalog.
3. It writes to a per-user branch
   (`pankosmia-edit-<github-login>`) via the GitHub Contents
   API, using the GitHub App's installation token.
4. A pull request is opened automatically against the repo's
   default branch.
5. A reviewer can approve and merge via the `/admin/*` endpoints.

Users never interact with GitHub directly. The server handles
branching, committing, and PR creation transparently.

---

## Configuration

| Env var | Default | Purpose |
|---------|---------|---------|
| `PANKOSMIA_MAX_USER_LANGUAGES` | `5` | Max languages a single user can claim |
| `PANKOSMIA_CATALOG_ORG` | — | GitHub org to discover language repos from |
| `PANKOSMIA_SQLITE_PATH` | — | Path to SQLite database for per-user state |

---

## Proxy rules

Add to your Netlify/proxy config:

```
/user-languages/*
```

This prefix must be proxied to the backend alongside the
existing `/auth/*`, `/burrito/*`, `/settings/*`, etc. rules.
