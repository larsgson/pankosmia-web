# pankosmia-docker

Hosted Pankosmia server: GitHub-backed multi-language
Bible-translation collaboration. Rust / Rocket / git2.

A single binary serves thousands of translators across hundreds of
language repos. Each language is a public GitHub repository;
translators sign in with their GitHub account and edit text through
their browser. The server forks, branches, pushes, and opens pull
requests on the user's behalf so translators never visit GitHub
directly. Audio assets stay outside git in object storage.

## Quick start (development)

```bash
# Build
cargo build

# Run with the default filesystem backend (no auth, single user):
cargo run -- /path/to/workspace

# Run with the GitHub backend (multi-user, hosted):
STORAGE_BACKEND=github \
  GITHUB_CLIENT_ID=...                                       \
  GITHUB_CLIENT_SECRET=...                                   \
  GITHUB_WEBHOOK_SECRET=...                                  \
  PANKOSMIA_TOKEN_ENCRYPTION_KEY=$(openssl rand -base64 32)  \
  PANKOSMIA_PUBLIC_ORIGIN=https://example.com                \
  PANKOSMIA_CATALOG_PATH=/path/to/catalog/languages.yaml     \
  cargo run -- /path/to/workspace
```

The default port is `19119`; configure via the standard
`ROCKET_PORT` env or `Rocket.toml`.

## Documentation

| Role | Read |
|---|---|
| Newcomer / architecture overview | `docs/ARCHITECTURE.md` |
| Operator running the server | `docs/HOSTING.md`, `docs/SCALING.md` |
| Building a JS/web client | `docs/CLIENT_INTEGRATION.md` |
| Setting up the language catalog | `docs/CATALOG_REPO_TEMPLATE.md` |
| Reviewing the data model | `docs/DATA_MODEL.md` |
| Reviewing security posture | `docs/SECURITY.md` |

## Tested on

- Linux x86_64 (Debian / Ubuntu): primary target.
- macOS (recent Apple silicon and Intel): supported for development.
- Windows: best-effort; not the primary deployment target.

Toolchain: stable Rust (1.86+), OpenSSL 3.x system library, Git.
For development clients, Node 18+ and npm 10+.

## Attribution

Forked from
[`pankosmia/pankosmia-web`](https://github.com/pankosmia/pankosmia-web),
MIT-licensed. The fork relationship has been severed on GitHub;
this project is no longer affiliated with the Pankosmia
organization. Endpoint URLs and on-disk content layout retain
backwards compatibility with `pankosmia/pankosmia-web` v0.14.x
clients where practical, so the same JS/React clients can drive
either server with minimal changes.

See `LICENSE` for full attribution.

## License

MIT — see `LICENSE`.
