//! Round-trip tests for `FsLanguageStore`.
//!
//! For each entity exposed by `ProjectStore`, write something and
//! read it back. These are the smoke tests that prove the FS impl is
//! self-consistent before we start refactoring endpoints to use it.
//!
//! Storage layout assumptions are checked by the `paths` unit tests
//! in `src/store/fs/paths.rs`; this file exercises the trait surface.

use pankosmia_docker::identity::{LanguageCode, UserId};
use pankosmia_docker::store::fs::FsLanguageStore;
use pankosmia_docker::store::project_store::ProjectStore;
use pankosmia_docker::store::types::{
    AppState, AuthRequest, Bcv, NewProject, NewRepo, Role,
};
use pankosmia_docker::structs::Typography;
use std::collections::BTreeMap;
use tempfile::TempDir;

fn fr() -> LanguageCode {
    LanguageCode::parse("fr").unwrap()
}

fn alice() -> UserId {
    UserId(uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap())
}

#[tokio::test]
async fn fs_role_is_always_owner_in_single_tenant_mode() {
    // The "always-Owner" property is the back-compat trick that lets
    // RequireRole<L> guards stay enabled on endpoint code without
    // breaking single-tenant deployments. Lock it in with a test.
    let tmp = TempDir::new().unwrap();
    let store = FsLanguageStore::new(tmp.path().to_path_buf());
    let role = store.project_role(alice(), fr()).await.unwrap();
    assert_eq!(role, Some(Role::Owner));

    // Even for a user / language that have never been registered:
    let bob = UserId::new();
    let lang = LanguageCode::parse("zz").unwrap();
    let role = store.project_role(bob, lang).await.unwrap();
    assert_eq!(role, Some(Role::Owner));
}

#[tokio::test]
async fn fs_create_and_list_languages() {
    let tmp = TempDir::new().unwrap();
    let store = FsLanguageStore::new(tmp.path().to_path_buf());
    store
        .create_project(
            alice(),
            NewProject {
                language: fr(),
                display_name: "French".into(),
            },
        )
        .await
        .unwrap();

    let listing = store.list_user_languages(alice()).await.unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].language.as_str(), "fr");
    assert_eq!(listing[0].display_name, "French");
    assert_eq!(listing[0].role, Role::Owner);
}

#[tokio::test]
async fn fs_user_settings_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FsLanguageStore::new(tmp.path().to_path_buf());

    let typography = Typography {
        font_set: "noto".into(),
        size: "16".into(),
        direction: "rtl".into(),
        features: BTreeMap::new(),
    };

    store.put_typography(alice(), typography.clone()).await.unwrap();
    let got = store.get_typography(alice()).await.unwrap();
    assert_eq!(got.font_set, "noto");
    assert_eq!(got.size, "16");
    assert_eq!(got.direction, "rtl");

    // Languages preference round-trip.
    let langs = vec![fr(), LanguageCode::parse("en").unwrap()];
    store.put_languages(alice(), langs.clone()).await.unwrap();
    let got = store.get_languages(alice()).await.unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].as_str(), "fr");
    assert_eq!(got[1].as_str(), "en");
}

#[tokio::test]
async fn fs_app_state_and_bcv_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FsLanguageStore::new(tmp.path().to_path_buf());

    // Initial reads default sensibly when nothing's been written.
    let state = store.get_app_state(fr()).await.unwrap();
    assert_eq!(state.bcv.book_code, "TIT");
    let bcv = store.get_bcv(fr(), alice()).await.unwrap();
    assert_eq!(bcv.book_code, "TIT");

    // Write and read back.
    let new_bcv = Bcv {
        book_code: "JAS".into(),
        chapter: 2,
        verse: 5,
    };
    store
        .put_bcv(fr(), alice(), new_bcv.clone())
        .await
        .unwrap();
    let got = store.get_bcv(fr(), alice()).await.unwrap();
    assert_eq!(got.book_code, "JAS");
    assert_eq!(got.chapter, 2);
    assert_eq!(got.verse, 5);

    // Per-(user, language): a different user on the same language
    // sees the default, not Alice's value.
    let bob = UserId(uuid::Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap());
    let bobs_bcv = store.get_bcv(fr(), bob).await.unwrap();
    assert_eq!(bobs_bcv.book_code, "TIT");
}

#[tokio::test]
async fn fs_auth_token_round_trip_and_validation() {
    let tmp = TempDir::new().unwrap();
    let store = FsLanguageStore::new(tmp.path().to_path_buf());

    // Empty key, traversal payloads, etc. rejected.
    assert!(store.put_auth_token(alice(), "", "x").await.is_err());
    assert!(store.put_auth_token(alice(), "..", "x").await.is_err());
    assert!(store.put_auth_token(alice(), "a/b", "x").await.is_err());
    assert!(store.put_auth_token(alice(), "a\0b", "x").await.is_err());

    // Round-trip a benign key.
    store
        .put_auth_token(alice(), "door43", "secret-code")
        .await
        .unwrap();
    let got = store.get_auth_token(alice(), "door43").await.unwrap();
    assert_eq!(got.as_deref(), Some("secret-code"));

    // Per-user isolation.
    let bob = UserId(uuid::Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap());
    let got = store.get_auth_token(bob, "door43").await.unwrap();
    assert_eq!(got, None);

    // Delete.
    store.delete_auth_token(alice(), "door43").await.unwrap();
    let got = store.get_auth_token(alice(), "door43").await.unwrap();
    assert_eq!(got, None);
}

#[tokio::test]
async fn fs_auth_request_take_is_one_shot() {
    let tmp = TempDir::new().unwrap();
    let store = FsLanguageStore::new(tmp.path().to_path_buf());

    let req = AuthRequest {
        code: "abc".into(),
        redirect_uri: "https://x".into(),
        timestamp: std::time::SystemTime::now(),
    };
    store
        .put_auth_request(alice(), "door43", req.clone())
        .await
        .unwrap();

    let taken = store
        .take_auth_request(alice(), "door43")
        .await
        .unwrap();
    assert!(taken.is_some());

    // Second take returns None.
    let taken_again = store.take_auth_request(alice(), "door43").await.unwrap();
    assert!(taken_again.is_none());
}

#[tokio::test]
async fn fs_repo_registry_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FsLanguageStore::new(tmp.path().to_path_buf());
    store
        .create_project(
            alice(),
            NewProject {
                language: fr(),
                display_name: "French".into(),
            },
        )
        .await
        .unwrap();

    let id = store
        .register_repo(
            fr(),
            NewRepo {
                name: "fr_jas".into(),
                flavor: Some("textTranslation".into()),
            },
        )
        .await
        .unwrap();

    // List shows it.
    let list = store.list_repos(fr()).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "fr_jas");
    assert_eq!(list[0].id, id);

    // Lookup by id.
    let found = store.lookup_repo(fr(), id).await.unwrap();
    assert_eq!(found.name, "fr_jas");

    // Working dir was created on disk.
    let working = std::path::Path::new(&found.working_path);
    assert!(working.is_dir(), "expected working dir to exist");

    // Unregister removes from registry.
    store.unregister_repo(fr(), id).await.unwrap();
    assert_eq!(store.list_repos(fr()).await.unwrap().len(), 0);
}

#[tokio::test]
async fn fs_repo_registration_rejects_traversal_in_name() {
    let tmp = TempDir::new().unwrap();
    let store = FsLanguageStore::new(tmp.path().to_path_buf());
    let bad = store
        .register_repo(
            fr(),
            NewRepo {
                name: "../escape".into(),
                flavor: None,
            },
        )
        .await;
    assert!(bad.is_err());
}

#[tokio::test]
async fn fs_app_state_put_then_get_returns_what_was_written() {
    let tmp = TempDir::new().unwrap();
    let store = FsLanguageStore::new(tmp.path().to_path_buf());
    store
        .put_app_state(
            fr(),
            AppState {
                bcv: Bcv {
                    book_code: "ROM".into(),
                    chapter: 1,
                    verse: 1,
                },
            },
        )
        .await
        .unwrap();
    let got = store.get_app_state(fr()).await.unwrap();
    assert_eq!(got.bcv.book_code, "ROM");
}
