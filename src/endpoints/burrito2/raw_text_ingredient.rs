use crate::gitea::{resolve_read_source, CuratedOrgs, GiteaProxyClient, ReadSource};
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

pub struct AcceptsEventStream;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AcceptsEventStream {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let accept = match req.headers().get_one("Accept") {
            Some(s) => s,
            None => return Outcome::Forward(Status::NotAcceptable),
        };
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

fn guess_content_type(ipath: &str) -> ContentType {
    let mut split_ipath = ipath.split('.');
    let mut suffix = "unknown";
    if let Some(_) = split_ipath.next() {
        if let Some(second) = split_ipath.next() {
            suffix = second;
        }
    }
    match mime_types().get(suffix) {
        Some(t) => t.clone(),
        None => ContentType::new("application", "text/plain"),
    }
}

#[get("/ingredient/raw/<repo_path..>?<ipath>", rank = 2)]
pub async fn raw_text_ingredient(
    store: &State<SharedProjectStore>,
    curated: &State<CuratedOrgs>,
    client: &State<GiteaProxyClient>,
    repo_path: PathBuf,
    ipath: String,
) -> status::Custom<(ContentType, String)> {
    if !check_path_string_components(ipath.clone()) {
        return not_ok_bad_repo_json_response();
    }
    match resolve_read_source(curated, &repo_path) {
        ReadSource::Gitea(parsed) => {
            match client
                .fetch_raw(&parsed.server, &parsed.org, &parsed.repo, &ipath, "master")
                .await
            {
                Ok((_content_type, bytes)) => match String::from_utf8(bytes) {
                    Ok(text) => status::Custom(Status::Ok, (guess_content_type(&ipath), text)),
                    Err(e) => not_ok_json_response(
                        Status::BadRequest,
                        make_bad_json_data_response(format!("not valid UTF-8: {}", e)),
                    ),
                },
                Err(e) => not_ok_json_response(
                    Status::BadGateway,
                    make_bad_json_data_response(format!("gitea proxy: {}", e)),
                ),
            }
        }
        ReadSource::LocalFilesystem => {
            let path_to_serve = match resolve_ingredient_path(store, &repo_path, &ipath) {
                Some(p) => p,
                None => return not_ok_bad_repo_json_response(),
            };
            match std::fs::read_to_string(path_to_serve) {
                Ok(v) => status::Custom(Status::Ok, (guess_content_type(&ipath), v)),
                Err(e) => not_ok_json_response(
                    Status::BadRequest,
                    make_bad_json_data_response(
                        format!("could not read ingredient content: {}", e).to_string(),
                    ),
                ),
            }
        }
    }
}

enum WatchMode {
    Gitea {
        server: String,
        org: String,
        repo: String,
        ipath: String,
        client: GiteaProxyClient,
    },
    Local {
        target_path: PathBuf,
        handle: crate::server::SubscriptionHandle,
        events: tokio::sync::broadcast::Receiver<crate::server::ChangeNotice>,
    },
    Error,
}

#[get("/ingredient/raw/<repo_path..>?<ipath>", rank = 1)]
pub async fn watch_text_ingredient(
    store: &State<SharedProjectStore>,
    curated: &State<CuratedOrgs>,
    _client: &State<GiteaProxyClient>,
    registry: &State<WatcherRegistry>,
    _accepts: AcceptsEventStream,
    repo_path: PathBuf,
    ipath: String,
) -> EventStream![Event + 'static] {
    let mode = match resolve_read_source(curated, &repo_path) {
        ReadSource::Gitea(parsed) => WatchMode::Gitea {
            server: parsed.server,
            org: parsed.org,
            repo: parsed.repo,
            ipath: ipath.clone(),
            client: GiteaProxyClient::new(),
        },
        ReadSource::LocalFilesystem => {
            let path_to_serve = resolve_ingredient_path(store, &repo_path, &ipath);
            match path_to_serve.as_deref().and_then(|p| {
                let target = std::path::Path::new(p).to_path_buf();
                registry.subscribe(&target).ok().map(|sub| (target, sub))
            }) {
                Some((target_path, (handle, events))) => WatchMode::Local {
                    target_path,
                    handle,
                    events,
                },
                None => WatchMode::Error,
            }
        }
    };

    EventStream! {
        match mode {
            WatchMode::Gitea { server, org, repo, ipath, client } => {
                let fetch = client.fetch_raw(&server, &org, &repo, &ipath, "master").await;
                let mut last_hash = match &fetch {
                    Ok((_, bytes)) => {
                        let mut hasher = Sha256::new();
                        hasher.update(bytes);
                        Some(format!("{:x}", hasher.finalize()))
                    }
                    Err(_) => None,
                };
                let initial_payload = serde_json::json!({
                    "hash": last_hash.clone().unwrap_or_default(),
                }).to_string();
                yield Event::data(initial_payload).event("change");

                let mut interval = time::interval(Duration::from_secs(30));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    let fetch = client.fetch_raw(&server, &org, &repo, &ipath, "master").await;
                    let new_hash = match &fetch {
                        Ok((_, bytes)) => {
                            let mut hasher = Sha256::new();
                            hasher.update(bytes);
                            Some(format!("{:x}", hasher.finalize()))
                        }
                        Err(_) => None,
                    };
                    if new_hash != last_hash {
                        last_hash = new_hash.clone();
                        let payload = serde_json::json!({
                            "hash": new_hash.unwrap_or_default(),
                        }).to_string();
                        yield Event::data(payload).event("change");
                    }
                    yield Event::comment("");
                }
            }
            WatchMode::Local { target_path, handle, mut events } => {
                let _handle = handle;
                let mut last_hash = sha256_of_file(&target_path);
                let initial_payload = serde_json::json!({
                    "hash": last_hash.clone().unwrap_or_default(),
                }).to_string();
                yield Event::data(initial_payload).event("change");

                let mut keepalive = time::interval(Duration::from_secs(20));
                keepalive.tick().await;

                loop {
                    tokio::select! {
                        got = events.recv() => {
                            match got {
                                Ok(_) => {}
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                                Err(_) => break,
                            }
                            time::sleep(Duration::from_millis(120)).await;
                            while let Ok(_) = events.try_recv() {}
                            let new_hash = sha256_of_file(&target_path);
                            if new_hash.is_none() {
                                continue;
                            }
                            if new_hash != last_hash {
                                last_hash = new_hash.clone();
                                let payload = serde_json::json!({
                                    "hash": new_hash.unwrap_or_default(),
                                }).to_string();
                                yield Event::data(payload).event("change");
                            }
                        }
                        _ = keepalive.tick() => {
                            yield Event::comment("");
                        }
                    }
                }
            }
            WatchMode::Error => {
                yield Event::data("invalid path or watcher init failed").event("error");
            }
        }
    }
}
