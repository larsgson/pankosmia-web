//! User language management endpoints.
//!
//! - `GET  /available-languages`       — all catalog languages
//! - `GET  /my-languages`              — user's claimed languages
//! - `POST /claim-language/<code>`     — claim a language
//! - `POST /release-language/<code>`   — release a language
//! - `GET  /current-language`          — active working language
//! - `POST /current-language/<code>`   — switch active language

use crate::auth::auth_user::AuthUser;
use crate::catalog::CatalogRegistry;
use crate::identity::LanguageCode;
use crate::store::sqlite_user_state::SqliteUserState;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_json_response, ok_ok_json_response};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, post, State};
use serde::Serialize;
use std::sync::Arc;

const DEFAULT_MAX_USER_LANGUAGES: usize = 5;

fn max_user_languages() -> usize {
    std::env::var("PANKOSMIA_MAX_USER_LANGUAGES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_USER_LANGUAGES)
}

#[derive(Serialize)]
struct AvailableLanguage {
    code: String,
    display_name: String,
    direction: Option<String>,
    script: Option<String>,
}

/// `GET /available-languages` — all languages in the catalog.
#[get("/available-languages")]
pub fn get_available_languages(
    catalog: &State<Arc<CatalogRegistry>>,
) -> status::Custom<(ContentType, String)> {
    let langs: Vec<AvailableLanguage> = catalog
        .list()
        .into_iter()
        .map(|r| AvailableLanguage {
            code: r.code.as_str().to_string(),
            display_name: r.display_name,
            direction: r.direction,
            script: r.script,
        })
        .collect();
    ok_json_response(serde_json::to_string_pretty(&langs).unwrap())
}

/// `GET /my-languages` — languages the current user has claimed.
#[get("/my-languages")]
pub fn get_my_languages(
    user: AuthUser,
    db: &State<Option<Arc<SqliteUserState>>>,
) -> status::Custom<(ContentType, String)> {
    let db = match db.inner().as_ref() {
        Some(d) => d,
        None => return ok_json_response("[]".to_string()),
    };
    match db.get_claimed_languages(&user.id) {
        Ok(langs) => {
            let codes: Vec<&str> = langs.iter().map(|l| l.as_str()).collect();
            ok_json_response(serde_json::to_string(&codes).unwrap())
        }
        Err(e) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("db: {}", e)),
        ),
    }
}

/// `POST /claim-language/<code>` — claim a language for editing.
#[post("/claim-language/<code>")]
pub fn claim_language(
    user: AuthUser,
    catalog: &State<Arc<CatalogRegistry>>,
    db: &State<Option<Arc<SqliteUserState>>>,
    code: &str,
) -> status::Custom<(ContentType, String)> {
    let lang = match LanguageCode::parse(code) {
        Ok(l) => l,
        Err(e) => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("invalid language code: {}", e)),
            )
        }
    };
    if !catalog.contains(&lang) {
        return not_ok_json_response(
            Status::NotFound,
            make_bad_json_data_response(format!("language {} not in catalog", code)),
        );
    }
    let db = match db.inner().as_ref() {
        Some(d) => d,
        None => {
            return not_ok_json_response(
                Status::ServiceUnavailable,
                make_bad_json_data_response("user state database not configured".into()),
            )
        }
    };
    let mut claimed = match db.get_claimed_languages(&user.id) {
        Ok(c) => c,
        Err(e) => {
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("db: {}", e)),
            )
        }
    };
    if claimed.iter().any(|c| c == &lang) {
        return ok_ok_json_response();
    }
    let max = max_user_languages();
    if claimed.len() >= max {
        return not_ok_json_response(
            Status::Forbidden,
            make_bad_json_data_response(format!("maximum {} languages per user reached", max)),
        );
    }
    claimed.push(lang.clone());
    if let Err(e) = db.put_claimed_languages(&user.id, &claimed) {
        return not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("db: {}", e)),
        );
    }
    if claimed.len() == 1 {
        let _ = db.put_current_language(&user.id, &lang);
    }
    ok_ok_json_response()
}

/// `POST /release-language/<code>` — release a claimed language.
#[post("/release-language/<code>")]
pub fn release_language(
    user: AuthUser,
    db: &State<Option<Arc<SqliteUserState>>>,
    code: &str,
) -> status::Custom<(ContentType, String)> {
    let lang = match LanguageCode::parse(code) {
        Ok(l) => l,
        Err(e) => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("invalid language code: {}", e)),
            )
        }
    };
    let db = match db.inner().as_ref() {
        Some(d) => d,
        None => {
            return not_ok_json_response(
                Status::ServiceUnavailable,
                make_bad_json_data_response("user state database not configured".into()),
            )
        }
    };
    let mut claimed = match db.get_claimed_languages(&user.id) {
        Ok(c) => c,
        Err(e) => {
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("db: {}", e)),
            )
        }
    };
    claimed.retain(|c| c != &lang);
    if let Err(e) = db.put_claimed_languages(&user.id, &claimed) {
        return not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("db: {}", e)),
        );
    }
    // If the released language was current, switch to first remaining or clear.
    if let Ok(Some(cur)) = db.get_current_language(&user.id) {
        if cur == lang {
            if let Some(first) = claimed.first() {
                let _ = db.put_current_language(&user.id, first);
            } else {
                let _ = db.clear_current_language(&user.id);
            }
        }
    }
    ok_ok_json_response()
}

/// `GET /current-language` — the user's active working language.
#[get("/current-language")]
pub fn get_current_language(
    user: AuthUser,
    db: &State<Option<Arc<SqliteUserState>>>,
) -> status::Custom<(ContentType, String)> {
    let db = match db.inner().as_ref() {
        Some(d) => d,
        None => return ok_json_response("null".to_string()),
    };
    match db.get_current_language(&user.id) {
        Ok(Some(lang)) => ok_json_response(format!("\"{}\"", lang.as_str())),
        Ok(None) => ok_json_response("null".to_string()),
        Err(e) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("db: {}", e)),
        ),
    }
}

/// `POST /current-language/<code>` — switch working language.
#[post("/current-language/<code>")]
pub fn post_current_language(
    user: AuthUser,
    db: &State<Option<Arc<SqliteUserState>>>,
    code: &str,
) -> status::Custom<(ContentType, String)> {
    let lang = match LanguageCode::parse(code) {
        Ok(l) => l,
        Err(e) => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("invalid language code: {}", e)),
            )
        }
    };
    let db = match db.inner().as_ref() {
        Some(d) => d,
        None => {
            return not_ok_json_response(
                Status::ServiceUnavailable,
                make_bad_json_data_response("user state database not configured".into()),
            )
        }
    };
    let claimed = match db.get_claimed_languages(&user.id) {
        Ok(c) => c,
        Err(e) => {
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("db: {}", e)),
            )
        }
    };
    if !claimed.iter().any(|c| c == &lang) {
        return not_ok_json_response(
            Status::Forbidden,
            make_bad_json_data_response(format!("language {} not in your claimed languages", code)),
        );
    }
    if let Err(e) = db.put_current_language(&user.id, &lang) {
        return not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("db: {}", e)),
        );
    }
    ok_ok_json_response()
}
