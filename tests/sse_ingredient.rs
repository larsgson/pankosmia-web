//! Integration tests for the dual-shape `/burrito/ingredient/raw/...`
//! endpoint:
//!
//!   * `Accept: text/event-stream` → SSE watch stream.
//!   * any other Accept (`*/*`, `text/plain`, etc.) → file bytes.
//!
//! The dispatch is what we test here. The watcher itself (notify →
//! coalesce → hash → emit `change`) is covered by manual smoke testing;
//! exercising it in-process would race against the OS's inotify queue.

use pankosmia_docker::endpoints::burrito2::raw_text_ingredient::{
    raw_text_ingredient, watch_text_ingredient,
};
use pankosmia_docker::identity::LanguageCode;
use pankosmia_docker::server::WatcherRegistry;
use pankosmia_docker::store::{fs::FsLanguageStore, SharedProjectStore};
use pankosmia_docker::structs::{
    AppSettings, Bcv, ProductSpec, ProjectIdentifier, Typography,
};
use rocket::http::{Accept, ContentType, Status};
use rocket::local::asynchronous::Client;
use rocket::routes;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn settings_with_repo_dir(p: &std::path::Path) -> AppSettings {
    AppSettings {
        working_dir: p.to_string_lossy().into_owned(),
        repo_dir: Mutex::new(p.to_string_lossy().into_owned()),
        app_resources_dir: p.to_string_lossy().into_owned(),
        languages: Mutex::new(vec!["en".into()]),
        gitea_endpoints: BTreeMap::new(),
        auth_tokens: Mutex::new(BTreeMap::new()),
        auth_requests: Mutex::new(BTreeMap::new()),
        bcv: Mutex::new(Bcv {
            book_code: "TIT".into(),
            chapter: 1,
            verse: 1,
        }),
        typography: Mutex::new(Typography {
            font_set: "default".into(),
            size: "14".into(),
            direction: "ltr".into(),
            features: BTreeMap::new(),
        }),
        current_project: Mutex::new(None::<ProjectIdentifier>),
        product: ProductSpec {
            name: "test".into(),
            short_name: "t".into(),
            version: "0".into(),
            date_time: "0".into(),
        },
        client_config: BTreeMap::new(),
        default_language: LanguageCode::parse("en").unwrap(),
    }
}

async fn make_client(repo_root: &std::path::Path) -> Client {
    let store: SharedProjectStore = Arc::new(FsLanguageStore::new(repo_root.to_path_buf()));
    let r = rocket::build()
        .manage(settings_with_repo_dir(repo_root))
        .manage(store)
        .manage(WatcherRegistry::new())
        .mount(
            "/burrito",
            routes![watch_text_ingredient, raw_text_ingredient],
        );
    Client::tracked(r).await.expect("rocket client")
}

#[rocket::async_test]
async fn read_handler_serves_file_for_wildcard_accept() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("_local_/_local_/r");
    let ing = repo.join("ingredients");
    std::fs::create_dir_all(&ing).unwrap();
    std::fs::write(ing.join("t.txt"), "hello\n").unwrap();

    let client = make_client(tmp.path()).await;
    let resp = client
        .get("/burrito/ingredient/raw/_local_/_local_/r?ipath=t.txt")
        .header(Accept::Any)
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let body = resp.into_string().await.unwrap();
    assert_eq!(body, "hello\n");
}

#[rocket::async_test]
async fn read_handler_serves_file_for_text_plain() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("_local_/_local_/r");
    let ing = repo.join("ingredients");
    std::fs::create_dir_all(&ing).unwrap();
    std::fs::write(ing.join("t.txt"), "hello\n").unwrap();

    let client = make_client(tmp.path()).await;
    let resp = client
        .get("/burrito/ingredient/raw/_local_/_local_/r?ipath=t.txt")
        .header(Accept::Plain)
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
}

#[rocket::async_test]
async fn watch_handler_dispatches_for_explicit_event_stream_accept() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("_local_/_local_/r");
    let ing = repo.join("ingredients");
    std::fs::create_dir_all(&ing).unwrap();
    std::fs::write(ing.join("t.txt"), "hello\n").unwrap();

    let client = make_client(tmp.path()).await;
    let resp = client
        .get("/burrito/ingredient/raw/_local_/_local_/r?ipath=t.txt")
        .header(Accept::EventStream)
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    assert_eq!(resp.content_type(), Some(ContentType::EventStream));
    // The body is a long-lived SSE stream; we deliberately don't read it
    // here — the Status + Content-Type pair already proves the SSE
    // handler dispatched. The body contents (initial-hash + on-change
    // events) are verified by manual smoke testing where racing the
    // OS-level inotify queue is acceptable.
    drop(resp);
}
