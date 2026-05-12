# Hosting

Operator-facing guide. How to deploy, configure, and run
`pankosmia_docker` in production.

For the architecture and design rationale, see
`docs/ARCHITECTURE.md`. For client-side integration, see
`docs/CLIENT_INTEGRATION.md`. For capacity planning, see
`docs/SCALING.md`.

---

## 1. Two deployment modes

`pankosmia_docker` ships one binary that supports two backends:

| Mode | When to use | Configuration |
|---|---|---|
| **`STORAGE_BACKEND=fs`** (default) | Single-tenant desktop / dev / internal LAN. No auth. | Just point at a workspace dir. |
| **`STORAGE_BACKEND=github`** | Multi-tenant hosted. GitHub App (identity + writes), GitHub-backed content. | All env vars below. |

The endpoint surface is identical between modes. A client built
for one runs against the other (modulo authentication).

---

## 2. Required environment for the GitHub backend

```bash
STORAGE_BACKEND=github                                       # selector

# GitHub App — user identity (OAuth-style flow)
GITHUB_CLIENT_ID=...                                         # the App's Client ID
GITHUB_CLIENT_SECRET=...                                     # the App's Client Secret

# GitHub App — app-level credentials (installation tokens)
GITHUB_APP_ID=...                                            # numeric App ID
# Pick exactly one of:
GITHUB_APP_PRIVATE_KEY_PATH=/run/secrets/pankosmia-app.pem   # file with the .pem PEM contents
GITHUB_APP_PRIVATE_KEY="-----BEGIN RSA PRIVATE KEY-----\n…"  # or inline PEM (PaaS-friendly)
PANKOSMIA_DEFAULT_INSTALLATION_ID=...                        # fallback when a language has no override

# Webhooks
GITHUB_WEBHOOK_SECRET=...                                    # HMAC for catalog/language webhooks

# Server-held secrets (generate once, persist; restarts invalidate
# stored OAuth identity tokens and session cookies otherwise)
PANKOSMIA_TOKEN_ENCRYPTION_KEY=$(openssl rand -base64 32)    # 32 bytes, base64
ROCKET_SECRET_KEY=$(openssl rand -base64 32)                 # cookie signing

# Server
PANKOSMIA_PUBLIC_ORIGIN=https://example.com                  # for OAuth callback URL
PANKOSMIA_CATALOG_PATH=/data/.pankosmia/catalog/languages.yaml
ROCKET_ADDRESS=0.0.0.0
ROCKET_PORT=19119
```

Plus, for object-storage audio offload (when implemented):

```bash
S3_ENDPOINT=https://s3.example.com   # or supabase storage URL
S3_ACCESS_KEY=...
S3_SECRET_KEY=...
S3_BUCKET=pankosmia-audio
```

Optional:

```bash
DATABASE_MAX_CONNECTIONS=10                # if a SQL backend is enabled
SUPABASE_JWKS_URL=...                      # vestigial; only if JWT auth is layered
SUPABASE_JWT_AUDIENCE=authenticated        # vestigial
```

---

## 3. Setting up the GitHub App

Once per deployment.

### 3.1 Create the App

On github.com, **Settings → Developer settings → GitHub Apps → New
GitHub App** (not "OAuth Apps").

| Field | Value |
|---|---|
| GitHub App name | anything unique; users see it on the consent screen and it appears as the commit author |
| Homepage URL | `https://<your-server>` (the public origin) |
| Callback URL | `https://<your-server>/auth/callback` |
| Request user authorization (OAuth) during installation | **unchecked** (the app initiates OAuth from a sign-in button, not at install time) |
| Webhook | **unchecked** — we use repo-level webhooks separately (see §6); the App's webhook isn't needed |
| Repository permissions | **Contents: Read and write**, **Pull requests: Read and write**, **Metadata: Read** |
| Where can this GitHub App be installed? | "Only on this account" if all language repos live under one org; "Any account" if scattered |

After creating:

1. Note the **App ID** (numeric, at the top of the App settings).
2. **Generate a private key** — downloads a `.pem` file. Store
   under e.g. `/run/secrets/pankosmia-app.pem` with `chmod 600`,
   set `GITHUB_APP_PRIVATE_KEY_PATH` to that path.
3. Under "OAuth credentials": note the **Client ID** and generate
   a **Client secret**. Set `GITHUB_CLIENT_ID` and
   `GITHUB_CLIENT_SECRET`.

The App requests **no scopes** for user authorization — the user
is granting identity only, no repo access. All repo writes happen
under the App's own identity via installation tokens.

### 3.2 Install on each language repo (or org)

For every account / org that owns language repos in your catalog:

1. App settings → **Install App** → pick the account → either
   "All repositories" or "Only select repositories" → tick the
   language repos.
2. After install, the URL ends in `/installations/<NNN>` — note the
   numeric **installation ID**.

Wire installations to languages via the catalog (one of):

- Set `PANKOSMIA_DEFAULT_INSTALLATION_ID=<NNN>` for the common
  case where one installation covers every language repo in the
  catalog.
- Add `installation_id: <NNN>` per `languages.yaml` entry for
  multi-org deployments where each language is owned by a different
  account.

Both can coexist: the per-language value overrides the default.

---

## 4. Setting up the catalog repo

The catalog repo (`pankosmia-org/catalog` by convention; pick any
name your org owns) is the registry of registered languages.

See `docs/CATALOG_REPO_TEMPLATE.md` for the full setup, including:

- Branch protection rules.
- The `validate-catalog` GitHub Action.
- The PR template prompting registrants for identity / repo
  details.
- The vetting checklist for the catalog admin.

Once the catalog repo exists with at least one language entry,
clone it into the workspace and point the server at it:

```bash
git clone https://github.com/pankosmia-org/catalog.git \
  /data/.pankosmia/catalog
export PANKOSMIA_CATALOG_PATH=/data/.pankosmia/catalog/languages.yaml
```

Or set `PANKOSMIA_CATALOG_REPO=pankosmia-org/catalog` to have the
server clone and refresh it autonomously (planned).

---

## 5. Reverse proxy configuration

`pankosmia_docker` does not terminate TLS. Put it behind Caddy /
nginx / similar.

### Caddy

```caddyfile
example.com {
    reverse_proxy /notifications/* 127.0.0.1:19119 {
        flush_interval -1
        transport http {
            read_timeout 1h
        }
    }
    reverse_proxy /burrito/ingredient/raw/* 127.0.0.1:19119 {
        flush_interval -1
        transport http {
            read_timeout 1h
        }
    }
    reverse_proxy 127.0.0.1:19119
}
```

### nginx

```nginx
location /notifications/ {
    proxy_pass http://127.0.0.1:19119;
    proxy_http_version 1.1;
    proxy_set_header Connection "";
    proxy_buffering off;
    proxy_cache off;
    proxy_read_timeout 1h;
    chunked_transfer_encoding off;
}

location /burrito/ingredient/raw/ {
    proxy_pass http://127.0.0.1:19119;
    proxy_http_version 1.1;
    proxy_set_header Connection "";
    proxy_buffering off;
    proxy_cache off;
    proxy_read_timeout 1h;
    chunked_transfer_encoding off;
}
```

### Cloudflare

Cloudflare's edge buffers responses by default; bypass cache for
SSE URLs (Page Rule: Cache Level = Bypass on `/notifications/*`
and `/burrito/ingredient/raw/*`). The free tier has a 100s response
idle timeout; SSE through Cloudflare free tier disconnects every
100s. Browser auto-reconnect handles this but produces noisy logs.
Use Workers / Enterprise for longer idles.

---

## 6. Webhook setup

Two kinds of webhooks land at the server:

### 6.1 Catalog webhook

In the catalog repo's GitHub settings → Webhooks → Add webhook:

- **Payload URL**: `https://<your-server>/webhook/catalog`
- **Content type**: `application/json`
- **Secret**: the value of `GITHUB_WEBHOOK_SECRET`
- **SSL verification**: enabled
- **Events**: just `push`

### 6.2 Per-language webhooks

Each language repo sends webhooks to a different URL (the language
code is in the path):

- **Payload URL**: `https://<your-server>/webhook/language/<code>`
- **Secret**: same `GITHUB_WEBHOOK_SECRET`
- **Events**: `push` and `pull_request`

The catalog admin sets up the catalog webhook; each language admin
sets up their own. Without webhooks, the server falls back to a
periodic 15-minute fetch — slower propagation but the system stays
functional.

---

## 7. File-descriptor limits

Hosted deployments will keep many SSE connections open; raise the
per-process limit. In a systemd unit:

```ini
[Service]
LimitNOFILE=65535
```

In a Docker run:

```bash
docker run --ulimit nofile=65535:65535 ...
```

Verify at runtime: `cat /proc/<pid>/limits | grep "Max open files"`.

---

## 8. Persistent storage

The workspace directory holds:

- The local clone of the catalog repo.
- Per-language upstream caches (one git working tree per
  registered language).
- Per-user fork clones (one per `(github_user_id, language_code)`
  pair the user has edited).
- Encrypted user OAuth tokens.

Mount as a persistent volume in container deployments:

```dockerfile
VOLUME /data
CMD ["/app/bin/server", "/data"]
```

Disk capacity: roughly the sum of all language repos' clone sizes
plus active user fork clones. Most deployments need ~50–500 GB.
Rough envelope per `docs/SCALING.md` §5.

Backups: snapshot the volume nightly. Loss of the workspace is
recoverable — content is on GitHub — but loss of the encrypted
token store forces every active user to re-sign-in. The token
store can be rebuilt from sign-ins; nothing structurally important
is lost.

---

## 9. Authentication / authorization at the server

Authentication is via GitHub OAuth. The server exchanges the OAuth
code for an access token, stores the token AES-GCM-encrypted with
a key from `PANKOSMIA_TOKEN_ENCRYPTION_KEY`, and uses it on each
request that touches GitHub.

Authorization is via GitHub repo collaborator permissions. A user's
role on a language is whatever GitHub says it is for that language
repo, cached briefly server-side. Non-collaborators on a public
language repo get `Viewer` (the read-only baseline).

The catalog acts as the gate before any per-language lookup: a
request for a language not in the catalog returns 404 immediately,
regardless of whether a clone exists or whether the user has
GitHub access to the upstream repo.

---

## 10. Behaviour notes

1. **Initial-hash event on every (re)connect.** The SSE watch
   endpoint sends one `change` event with the current hash on
   every connection, including reconnects. Clients compare to
   their last-known hash; equal → nothing changed; different →
   refetch.
2. **Atomic-rename saves are tolerated.** During temp-file-then-
   rename, the file briefly disappears; the watcher coalesces and
   skips the missing-file moment.
3. **Watcher exits when client disconnects.** No lingering inotify
   subscriptions when an `EventSource` closes the tab. The shared
   watcher registry ensures one inotify subscription per file
   regardless of subscriber count.
4. **Token revocation propagates lazily.** When a user revokes the
   OAuth app on github.com, the next API call returns 401; the
   server clears the session; the client re-signs-in.
5. **Webhook-missed event safety net.** A 15-minute periodic
   fetch catches anything the webhook stream missed. Latency for
   propagation is bounded by that interval in the worst case.
6. **`/version` is unauthenticated by design.** Useful for
   liveness probes; keep it that way.

---

## 11. CORS for cross-origin clients

If clients are served from a different origin, configure CORS at
the reverse proxy (the server itself doesn't ship a CORS fairing
— that's an operator decision per deployment).

Required for credentialed `EventSource` and `fetch`:

```
Access-Control-Allow-Origin: <your-client-origin>   # NOT *
Access-Control-Allow-Credentials: true
Access-Control-Allow-Headers: X-Language-Code, Content-Type
Access-Control-Allow-Methods: GET, POST, DELETE, OPTIONS
```

Same-origin deployments don't need CORS at all.

---

## 12. Logging and observability

Logs to stdout in human-readable format by default. Structured
JSON output via `RUST_LOG` controls.

A `/metrics` Prometheus endpoint and `tracing` instrumentation are
planned.

What to monitor in production:

| Symptom | Look at |
|---|---|
| Latency spike on hot path | Blocking thread pool queue depth |
| SSE clients reconnecting in waves | Server graceful-shutdown logs; CPU usage |
| One language's writes are slow | Per-language lock wait time |
| Memory growing unbounded | Cache sizes; LRU eviction rate |
| inotify watch failures | `cat /proc/sys/fs/inotify/max_user_watches` |
| Audio uploads failing | Object storage quota / network egress |
| 401s mysteriously | Reverse-proxy `Authorization` pass-through |
| 5xx during deploys | Graceful-shutdown duration |
| GitHub API rate-limit headroom shrinking | Per-user token usage |

---

## 13. Reporting issues

Open issues at
[`larsgson/pankosmia-docker`](https://github.com/larsgson/pankosmia-docker/issues).
Include:

- Crate version (`pankosmia_docker --version`).
- Backend mode (`STORAGE_BACKEND` value).
- The endpoint and full URL involved.
- A curl reproduction if possible.
- Server log output around the failure.

For changes affecting the wire contract or the catalog-repo
schema, open a discussion before a PR.
