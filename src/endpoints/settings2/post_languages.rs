use crate::identity::{LanguageCode, COMPAT_USER};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::client::Clients;
use crate::utils::files::write_user_settings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::os_slash_str;
use crate::utils::response::{not_ok_json_response, ok_ok_json_response};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{post, State};
use std::path::PathBuf;

/// *`POST /languages/<lang>/<lang>/...`*
///
/// Typically mounted as **`/languages/<lang>/<lang>/...`**
///
/// Sets UI languages.
///
/// Dual-write during the M2 → M5 transition:
///   - persists through the `ProjectStore` trait (multi-tenant
///     authoritative storage from M5+),
///   - updates the `AppSettings` in-memory mirror (so the SSE
///     notifications stream sees the change),
///   - rewrites legacy `user_settings.json` for snapshot/back-compat.
#[post("/languages/<languages..>")]
pub async fn post_languages(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    clients: &State<Clients>,
    languages: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let language_strings: Vec<String> = languages
        .display()
        .to_string()
        .split(os_slash_str())
        .map(|s| s.to_string())
        .collect();
    if language_strings.is_empty() {
        return not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response("No language code found".to_string()),
        );
    }
    let mut typed_langs: Vec<LanguageCode> = Vec::with_capacity(language_strings.len());
    for s in &language_strings {
        match LanguageCode::parse(s) {
            Ok(l) => typed_langs.push(l),
            Err(_) => {
                return not_ok_json_response(
                    Status::BadRequest,
                    make_bad_json_data_response(format!("Bad language code: {}", s)),
                )
            }
        }
    }

    // Trait write — authoritative source going forward.
    if let Err(e) = store.put_languages(COMPAT_USER, typed_langs.clone()).await {
        return not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("Could not persist languages: {}", e)),
        );
    }

    // Mirror update — keeps the SSE notifications stream in sync.
    *state.languages.lock().unwrap() = language_strings;

    // Legacy snapshot — keep `user_settings.json` current for now.
    if let Err(e) = write_user_settings(&state, &clients) {
        return not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!("Could not write out user settings: {}", e)),
        );
    }
    ok_ok_json_response()
}
