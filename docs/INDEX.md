# Documentation index

pankosmia-docker is the online hosted version of the
[Pankosmia](https://pankosmia.dev/) platform
([GitHub](https://github.com/pankosmia)) — a hosted Scripture
Burrito read/write service where every edit lands as a GitHub
Pull Request against the language's source-of-truth repo, with
sign-in, admin review, change notifications, and rate-limiting
as the supporting infrastructure that makes this safe in a
multi-user setting.

This project retains API compatibility with the offline
[pankosmia-web](https://github.com/pankosmia/pankosmia-web)
desktop app. The same client apps (munchers) work with both.

---

## For client developers

| Doc | What it covers |
|-----|----------------|
| [CLIENT_INTEGRATION.md](CLIENT_INTEGRATION.md) | Server contract: endpoints, save/read patterns, SSE, errors |
| [USER_LANGUAGES.md](USER_LANGUAGES.md) | User language management: claim, switch, and the dashboard's role |
| [API_ROUTES.md](API_ROUTES.md) | Complete HTTP endpoint reference for proxy config and integration |

## For operators

| Doc | What it covers |
|-----|----------------|
| [HOSTING.md](HOSTING.md) | Environment variables, GitHub App setup, reverse proxy, deployment |

## For contributors (`dev/`)

| Doc | What it covers |
|-----|----------------|
| [dev/ARCHITECTURE.md](dev/ARCHITECTURE.md) | System design, trust topology, storage layout |
| [dev/DECISIONS.md](dev/DECISIONS.md) | Why things are shaped the way they are (D1-D7) |
| [dev/DATA_MODEL.md](dev/DATA_MODEL.md) | Entity catalog: who lives where, who owns what |
| [dev/SCALING.md](dev/SCALING.md) | Capacity planning, locking, thread pools |
| [dev/SECURITY.md](dev/SECURITY.md) | Threat model and defenses |
| [dev/CATALOG_REPO_TEMPLATE.md](dev/CATALOG_REPO_TEMPLATE.md) | Setting up the language catalog repo |
| [dev/CLIENT_WRAPPER_GUIDE.md](dev/CLIENT_WRAPPER_GUIDE.md) | Building a thin React wrapper (muncher) on top of the API |
| [dev/PER_LANGUAGE_USER_STATE.md](dev/PER_LANGUAGE_USER_STATE.md) | Per-language user state design |

## Implementation specs (`impl/`)

| Doc | Status |
|-----|--------|
| [impl/AUDIO_STRATEGY.md](impl/AUDIO_STRATEGY.md) | Shipped |
| [impl/BULK_OPS.md](impl/BULK_OPS.md) | Shipped |
| [impl/USER_STATE_FUTURE.md](impl/USER_STATE_FUTURE.md) | Deferred |

## Suggested reading order

1. This file (you're here).
2. **CLIENT_INTEGRATION.md** — what clients see.
3. **USER_LANGUAGES.md** — how users pick a language.
4. **API_ROUTES.md** — every endpoint at a glance.
5. **HOSTING.md** — running it.
6. **dev/ARCHITECTURE.md** — how it works internally.
