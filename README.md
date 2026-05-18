# pankosmia-docker

The online hosted version of the
[Pankosmia](https://pankosmia.dev/) platform
([GitHub](https://github.com/pankosmia)).

A hosted [Scripture Burrito](https://docs.burrito.bible/) read/write
service where every edit lands as a GitHub Pull Request against the
language's source-of-truth repo. Sign-in, admin review, change
notifications, and rate-limiting make this safe for multi-user
collaboration.

Built on [Rocket](https://rocket.rs/) (Rust) and
[git2](https://crates.io/crates/git2).

## The Pankosmia platform

This project is part of the [Pankosmia](https://pankosmia.dev/)
ecosystem — a set of tools for collaborative Bible translation
built around the [Scripture Burrito](https://docs.burrito.bible/)
standard. The offline Pankosmia desktop app
([pankosmia-web](https://github.com/pankosmia/pankosmia-web))
runs a local server on the user's machine with a filesystem
backend. **pankosmia-docker** takes the same API surface online:
it replaces the filesystem with GitHub as the single source of
truth, adds multi-user authentication, and deploys as a Docker
container.

The same React/JS client apps (munchers) that run inside the
desktop Pankosmia shell can drive this hosted service with
minimal changes. Content is stored in Scripture Burrito format in
both cases.

For more about the Pankosmia project:
- Website: **https://pankosmia.dev/**
- GitHub org: **https://github.com/pankosmia**

## What it does

Translators open a web app in their browser, pick a language, and
edit Scripture or OBS content. Under the hood the server:

1. **Authenticates** the user via GitHub OAuth.
2. **Reads** content from the language's GitHub repo.
3. **Writes** edits to a per-user branch using the GitHub App
   installation token.
4. **Opens a Pull Request** automatically so a reviewer can approve
   and merge the change into the canonical branch.

No one needs to know git. The server handles branching, committing,
and PR lifecycle transparently.


## Architecture at a glance

```
Browser (Netlify)          Server (Railway)           GitHub
┌──────────────┐    HTTPS   ┌──────────────┐    API   ┌──────────────┐
│  Dashboard   │───────────>│  pankosmia-  │────────>│ pankosmia-   │
│  + Munchers  │<───────────│  docker      │<────────│ langs/yua    │
│  (static JS) │    JSON    │  (Rust)      │   PRs   │ langs/fr     │
└──────────────┘            └──────────────┘         │ langs/...    │
                                   │                 └──────────────┘
                                   │
                              ┌────┴────┐
                              │ SQLite  │  per-user state
                              │ /data/  │  (languages, settings)
                              └─────────┘
```

**Key components:**
- **Catalog** — the server discovers available languages from the
  `pankosmia-langs` GitHub org (repos tagged `pankosmia-language`).
- **User language management** — each user claims languages to work
  on; the server tracks the active language and routes reads/writes
  to the correct repo.
- **Edit flow** — writes go through the GitHub Contents API to a
  per-user branch, with automatic PR creation.
- **Admin review** — reviewers approve or reject PRs via dedicated
  endpoints.

## Quick start

```bash
GITHUB_APP_ID=... \
  GITHUB_CLIENT_ID=... \
  GITHUB_CLIENT_SECRET=... \
  PANKOSMIA_CATALOG_ORG=pankosmia-langs \
  cargo run -- /path/to/workspace
```

Default port: `19119` (override via `ROCKET_PORT`).

See [`docs/HOSTING.md`](docs/HOSTING.md) for the full environment
variable reference and deployment guide.

## Documentation

See [`docs/INDEX.md`](docs/INDEX.md) for the full reading guide.

| I want to... | Read |
|---|---|
| Understand the system | [`docs/dev/ARCHITECTURE.md`](docs/dev/ARCHITECTURE.md) |
| Run / deploy the server | [`docs/HOSTING.md`](docs/HOSTING.md) |
| Build a client app | [`docs/CLIENT_INTEGRATION.md`](docs/CLIENT_INTEGRATION.md) |
| Integrate user language switching | [`docs/USER_LANGUAGES.md`](docs/USER_LANGUAGES.md) |
| See all API routes | [`docs/API_ROUTES.md`](docs/API_ROUTES.md) |

## License

MIT — see `LICENSE`.

Forked from
[`pankosmia/pankosmia-web`](https://github.com/pankosmia/pankosmia-web).
