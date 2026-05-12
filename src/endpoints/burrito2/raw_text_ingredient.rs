use crate::server::WatcherRegistry;
use crate::store::SharedProjectStore;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::mime::mime_types;
use crate::utils::paths::{check_path_components, check_path_string_components, os_slash_str};
use crate::utils::response::{not_ok_bad_repo_json_response, not_ok_json_response};
use rocket::http::{ContentType, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::status;
use rocket::response::stream::{Event, EventStream};
use rocket::tokio;
use rocket::tokio::time::{self, Duration};
use rocket::{get, State};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Request guard that succeeds only when the client's `Accept` header
/// explicitly lists `text/event-stream`. Rejects `*/*` and other
/// wildcards. We forward (not error) on miss so Rocket tries the next
/// matching route — this is what lets one URL serve both the file bytes
/// (rank = 2) and an SSE stream when the client really asks for one.
pub struct AcceptsEventStream;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AcceptsEventStream {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let accept = match req.headers().get_one("Accept") {
            Some(s) => s,
            None => return Outcome::Forward(Status::NotAcceptable),
        };
        // Only match an explicit text/event-stream entry. Wildcards
        // (`*/*`, `text/*`) are NOT enough — they'd let curl's default
        // hit this route, which is exactly the bug we're avoiding.
        let asks_for_sse = accept
            .split(',')
            .map(|s| s.trim().split(';').next().unwrap_or("").trim())
            .any(|s| s.eq_ignore_ascii_case("text/event-stream"));
        if asks_for_sse {
            Outcome::Success(AcceptsEventStream)
        } else {
            Outcome::Forward(Status::NotAcceptable)
        }
    }
}

// This URL has TWO response shapes, dispatched by the `Accept` header:
//
//   * `Accept: text/event-stream` (sent automatically by `EventSource`)
//     → `watch_text_ingredient` returns an SSE stream of `change` events.
//   * Anything else (curl, browser address bar, regular fetch)
//     → `raw_text_ingredient` returns the file bytes (rank = 2 fallback).
//
// Both handlers live here. Don't move one without the other.
//
// `watch_text_ingredient` is unauthenticated by design — auth/CORS for
// hosted deployments belongs in fairings on the hosting layer (see
// docs/HOSTING.md), not here.

/// Resolve `(repo_path, ipath)` to an absolute path under `repo_dir`,
/// rejecting traversal attempts. Returns `None` if either segment fails
/// validation. Shared by both the read and watch handlers.
fn resolve_ingredient_path(
    store: &State<SharedProjectStore>,
    repo_path: &PathBuf,
    ipath: &str,
) -> Option<String> {
    let path_components = repo_path.components();
    if !check_path_components(&mut path_components.clone()) {
        return None;
    }
    if !check_path_string_components(ipath.to_string()) {
        return None;
    }
    Some(
        store.workspace_root().to_string_lossy().into_owned()
            + os_slash_str()
            + &repo_path.display().to_string()
            + "/ingredients/"
            + ipath,
    )
}

fn sha256_of_file(p: &Path) -> Option<String> {
    let bytes = std::fs::read(p).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(format!("{:x}", hasher.finalize()))
}

/// *`GET /ingredient/raw/<repo_path>?ipath=my_burrito_path`*
///
/// Typically mounted as **`/burrito/ingredient/raw/<repo_path>?ipath=my_burrito_path`**.
///
/// Companion: this same URL serves an SSE stream when the request includes
/// `Accept: text/event-stream` — see `watch_text_ingredient`.
///
/// Returns a raw text resource. We try to guess the mimetype.
#[get("/ingredient/raw/<repo_path..>?<ipath>", rank = 2)]
pub async fn raw_text_ingredient(
    store: &State<SharedProjectStore>,
    repo_path: PathBuf,
    ipath: String,
) -> status::Custom<(ContentType, String)> {
    let path_to_serve = match resolve_ingredient_path(store, &repo_path, &ipath) {
        Some(p) => p,
        None => return not_ok_bad_repo_json_response(),
    };
    match std::fs::read_to_string(path_to_serve) {
        Ok(v) => {
            let mut split_ipath = ipath.split(".").clone();
            let mut suffix = "unknown";
            if let Some(_) = split_ipath.next() {
                if let Some(second) = split_ipath.next() {
                    suffix = second;
                }
            }
            status::Custom(
                Status::Ok,
                (
                    match mime_types().get(suffix) {
                        Some(t) => t.clone(),
                        None => ContentType::new("application", "text/plain"),
                    },
                    v,
                ),
            )
        }
        Err(e) => not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(
                format!("could not read ingredient content: {}", e).to_string(),
            ),
        ),
    }
}

/// *`GET /ingredient/raw/<repo_path>?ipath=my_burrito_path`* with
/// `Accept: text/event-stream` — opens an SSE stream emitting `change`
/// events whenever the file's SHA-256 hash changes.
///
/// Companion: same URL without that header serves the file bytes.
///
/// On (re)connect emits one `change` event with the current hash, then
/// further `change` events on file modifications. 20s keepalive comments.
/// No `Last-Event-ID` resume in v1 — the initial-hash event lets clients
/// detect "did anything change while disconnected?" by comparing.
#[get("/ingredient/raw/<repo_path..>?<ipath>", rank = 1)]
pub async fn watch_text_ingredient(
    store: &State<SharedProjectStore>,
    registry: &State<WatcherRegistry>,
    _accepts: AcceptsEventStream,
    repo_path: PathBuf,
    ipath: String,
) -> EventStream![Event + 'static] {
    // Resolve up front so a bad path closes the stream immediately
    // rather than silently watching nothing.
    let path_to_serve = resolve_ingredient_path(store, &repo_path, &ipath);

    // Subscribe to the shared registry. One inotify per (dir, file)
    // regardless of subscriber count — the M3.5 fan-out fix.
    let subscription = path_to_serve.as_deref().and_then(|p| {
        let target = std::path::Path::new(p).to_path_buf();
        registry.subscribe(&target).ok().map(|sub| (target, sub))
    });

    EventStream! {
        let (target_path, (handle, mut events)) = match subscription {
            Some(s) => s,
            None => {
                yield Event::data("invalid path or watcher init failed").event("error");
                return;
            }
        };
        // The handle keeps the shared subscription alive for the
        // duration of this stream. Dropped when the EventStream
        // ends or the client disconnects.
        let _handle = handle;

        // Initial hash on (re)connect.
        let mut last_hash = sha256_of_file(&target_path);
        let initial_payload = serde_json::json!({
            "hash": last_hash.clone().unwrap_or_default(),
        })
        .to_string();
        yield Event::data(initial_payload).event("change");

        // Coalesce inotify bursts: drain the channel for ~120ms
        // after the first event, then hash once. 20s keepalive
        // comments otherwise.
        let mut keepalive = time::interval(Duration::from_secs(20));
        keepalive.tick().await; // skip the immediate first tick

        loop {
            tokio::select! {
                got = events.recv() => {
                    match got {
                        Ok(_) => {}
                        // Lagged: we missed events. Treat as a
                        // change signal — the next hash compare
                        // catches up.
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => break,
                    }
                    time::sleep(Duration::from_millis(120)).await;
                    // Drain anything else that arrived in the window.
                    while let Ok(_) = events.try_recv() {}
                    let new_hash = sha256_of_file(&target_path);
                    if new_hash.is_none() {
                        // File-missing window during atomic rename
                        // — skip and wait for the next event.
                        continue;
                    }
                    if new_hash != last_hash {
                        last_hash = new_hash.clone();
                        let payload = serde_json::json!({
                            "hash": new_hash.unwrap_or_default(),
                        })
                        .to_string();
                        yield Event::data(payload).event("change");
                    }
                }
                _ = keepalive.tick() => {
                    yield Event::comment("");
                }
            }
        }
    }
}
