# API route reference

Complete list of HTTP endpoints exposed by `pankosmia_docker`.
Generated from `src/utils/launch.rs` (route registration) and the
`#[get]` / `#[post]` annotations in each endpoint file.

Use this to configure reverse proxy rules (Netlify, nginx, etc.)
and as a quick reference for client developers. For detailed
request/response semantics see `CLIENT_INTEGRATION.md`.

---

## Root-mounted (`/`)

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/` | Redirect to dashboard |
| GET | `/favicon.ico` | Favicon |
| GET | `/list-clients` | List registered clients |
| GET | `/client-interfaces` | Client interface definitions |
| GET | `/client-config` | Client configuration |
| GET | `/version` | Server version |
| GET | `/health` | Healthcheck (200 when ready, 503 otherwise) |
| GET | `/auth/start?redirect=...` | Start GitHub OAuth flow |
| GET | `/auth/callback?code=...&state=...` | OAuth callback (GitHub redirects here) |
| POST | `/auth/logout` | Clear session |
| GET | `/me` | Current user's GitHub profile |
| POST | `/webhook/catalog` | Catalog webhook (GitHub → server, not browser-facing) |
| POST | `/webhook/language/<code>` | Per-language webhook (GitHub → server, not browser-facing) |

## `/notifications`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/notifications` | SSE event stream (content change notifications) |

## `/settings`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/settings/languages` | Get user's selected languages |
| POST | `/settings/languages/<languages..>` | Set selected languages |
| GET | `/settings/auth-token/<token_key>/<code>/<client_code>` | Exchange auth token |
| GET | `/settings/typography` | Get typography settings |
| POST | `/settings/typography/<font_set>/<size>/<direction>` | Set typography |
| POST | `/settings/typography-feature/<font_name>/<feature>/<new_value>` | Set a typography feature |

## `/navigation`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/navigation/bcv` | Get current BCV cursor |
| POST | `/navigation/bcv/<book_code>/<chapter>/<verse>` | Set BCV cursor |

## `/app-state`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/app-state/current-project` | Get current project |
| POST | `/app-state/current-project/<source>/<org>/<project>` | Set current project |
| POST | `/app-state/current-project` | Clear current project |

## `/i18n`

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/i18n/` | Post i18n data (JSON body) |
| GET | `/i18n/raw` | Raw i18n strings |
| GET | `/i18n/negotiated/<filter..>` | Negotiated i18n |
| GET | `/i18n/flat/<filter..>` | Flat i18n |
| GET | `/i18n/untranslated/<lang>` | Untranslated strings for a language |
| GET | `/i18n/used-languages` | Languages used in i18n |

## `/burrito`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/burrito/ingredient/raw/<repo_path..>?ipath=...` | Read text ingredient (+ SSE watch variant) |
| GET | `/burrito/ingredients/raw/<repo_path..>?ipath=...` | Read multiple text ingredients |
| GET | `/burrito/ingredient/bytes/<repo_path..>?ipath=...` | Read binary ingredient |
| GET | `/burrito/ingredient/zipped/<repo_path..>?ipath=...` | Read ingredient(s) as zip |
| POST | `/burrito/ingredient/raw/<repo_path..>?ipath=...&update_ingredients&no_bak` | Write text ingredient |
| POST | `/burrito/ingredient/bytes/<repo_path..>?ipath=...` | Write binary ingredient (multipart) |
| POST | `/burrito/ingredient/zipped/<repo_path..>?ipath=...` | Write ingredient(s) from zip (multipart) |
| POST | `/burrito/ingredient/delete/<repo_path..>?ipath=...` | Delete one ingredient |
| POST | `/burrito/ingredients/delete/<repo_path..>?ipath=...` | Delete multiple ingredients |
| POST | `/burrito/ingredient/copy/<repo_path..>?src_path&target_path&delete_src` | Copy/move ingredient |
| POST | `/burrito/ingredient/revert/<repo_path..>?ipath=...` | Revert ingredient to HEAD |
| GET | `/burrito/metadata/raw/<repo_path..>` | Raw metadata.json |
| GET | `/burrito/metadata/summary/<repo_path..>` | Summary metadata |
| GET | `/burrito/metadata/summaries?org=...` | All metadata summaries |
| POST | `/burrito/metadata/remake-ingredients/<repo_path..>` | Rebuild ingredients metadata |
| GET | `/burrito/paths/<repo_path..>` | List file paths in repo |
| GET | `/burrito/audit/<repo_path..>` | Audit a burrito repo |
| GET | `/burrito/zipped/<repo_path..>` | Download repo as zip |
| POST | `/burrito/zipped/<repo_path..>` | Upload repo from zip (multipart) |
| POST | `/burrito/remake_burrito_from_zip/<temp_id>/<repo_path..>` | Rebuild burrito from temp zip |

## `/git`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/git/list-local-repos` | List local repos |
| GET | `/git/status/<repo_path..>` | Git status |
| GET | `/git/log/<repo_path..>` | Git log |
| GET | `/git/branches/<repo_path..>` | List branches |
| GET | `/git/remotes/<repo_path..>` | List remotes |
| POST | `/git/clone-repo/<repo_path..>?branch=...` | Clone a repo |
| POST | `/git/delete/<repo_path..>` | Delete a local repo |
| POST | `/git/add-and-commit/<repo_path..>` | Add + commit (JSON body) |
| POST | `/git/push/<repo_path..>` | Push (JSON body) |
| POST | `/git/pull-repo/<remote_name>/<repo_path..>` | Pull from remote |
| POST | `/git/branch/<branch_ref>/<repo_path..>` | Switch branch |
| POST | `/git/new-branch/<branch_ref>/<repo_path..>` | Create + switch branch |
| POST | `/git/remote/add/<repo_path..>?remote_name&remote_url` | Add remote |
| POST | `/git/remote/delete/<repo_path..>?remote_name` | Delete remote |
| POST | `/git/copy/<repo_path..>?target_path&delete_src&add_ignore` | Copy/move repo |
| POST | `/git/new-text-translation` | Create text translation repo (JSON) |
| POST | `/git/new-bcv-resource` | Create BCV resource repo (JSON) |
| POST | `/git/new-bcv-resource-book/<repo_path..>` | Add scripture book (JSON) |
| POST | `/git/new-obs-resource` | Create OBS resource repo (JSON) |
| POST | `/git/new-scripture-book/<repo_path..>` | Add scripture book (JSON) |
| POST | `/git/new-tcore-resource` | Create tCore resource repo (JSON) |
| POST | `/git/new-translation-plan-resource` | Create translation plan repo (JSON) |

## `/gitea`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/gitea/remote-repos/<server>/<org>` | List org repos on a Gitea server |
| GET | `/gitea/user-remote-repos/<server>/<user>` | List user repos on Gitea |
| GET | `/gitea/endpoints` | Get configured Gitea endpoints |
| GET | `/gitea/login/<token_key>/<redir_path..>` | Gitea proxy login |
| GET | `/gitea/logout/<token_key>` | Gitea proxy logout |
| GET | `/gitea/my-collaborators/<proxy>/<org>/<project>` | List collaborators |

## `/content-utils`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/content-utils/templates` | List content templates |
| GET | `/content-utils/metadata-template/<name>` | Get a metadata template |
| GET | `/content-utils/template-filenames/<template>` | List template filenames |
| GET | `/content-utils/template/<name>/<filename>` | Get a template file |
| GET | `/content-utils/versifications` | List versifications |
| GET | `/content-utils/versification/<name>` | Get a versification |
| GET | `/content-utils/product?resource_path=...` | Product content catalog |

## `/admin`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/admin/pending-prs?language=...` | List pending PRs for review |
| GET | `/admin/pr-files?language&pr` | List files in a PR |
| POST | `/admin/approve?language&pr&method` | Approve + merge a PR |
| POST | `/admin/reject?language&pr&reason` | Reject (close) a PR |

## `/temp`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/temp/bytes/<temp_id>` | Read a temp file |
| POST | `/temp/bytes` | Write a temp file (multipart) |

## `/llm`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/llm/model` | List available LLM models |
| POST | `/llm/rag-prompt` | RAG prompt (JSON body) |

## `/video`

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/video/obs-para/<repo_path..>` | Generate OBS para video (JSON) |
| POST | `/video/obs-story/<repo_path..>` | Generate OBS story video (JSON) |

## `/net`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/net/status` | Network mode status |
| POST | `/net/enable` | Enable network |
| POST | `/net/disable` | Disable network |

## `/debug`

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/debug/status` | Debug mode status |
| GET | `/debug/enable` | Enable debug |
| GET | `/debug/disable` | Disable debug |

## Static file mounts

| Path prefix | Source |
|-------------|--------|
| `/webfonts/` | Webfont CSS + font files |
| `/app-resources/` | App resources (themes, product config) |
| `/clients/<name>/` | Per-client static build output |

---

## Reverse proxy notes

When proxying through Netlify, nginx, or similar:

- **Proxy all API prefixes** listed above as `status = 200`
  rewrites (transparent proxy, not redirects).
- **Do not proxy webhooks.** `/webhook/catalog` and
  `/webhook/language/<code>` are called by GitHub directly to the
  backend URL. They use HMAC signature validation
  (`GITHUB_WEBHOOK_SECRET`) and must hit the backend origin.
- **Static files** (`/webfonts/`, `/app-resources/`,
  `/clients/`) are served by the proxy itself (Netlify CDN) in a
  hosted deployment, not forwarded to the backend.
- **SSE** (`/notifications`) requires the proxy to support
  streaming responses. Netlify's proxy handles this; the backend
  sends a heartbeat comment every 15 seconds.

### Minimum proxy rules for a functional deployment

These prefixes cover the core editing workflow:

```
/auth/*          /me              /notifications
/burrito/*       /settings/*      /navigation/*
/app-state/*     /i18n/*          /git/*
/content-utils/* /admin/*         /temp/*
/list-clients    /client-interfaces  /client-config
/version         /health
```

Optional (feature-dependent):

```
/gitea/*         — only if Gitea integration is used
/llm/*           — only if LLM features are enabled
/video/*         — only if OBS video generation is used
/net/*           — network mode toggle
/debug/*         — debug mode toggle (consider omitting in production)
```
