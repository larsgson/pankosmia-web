use crate::identity::COMPAT_USER;
use crate::store::SharedProjectStore;
use crate::structs::{AppSettings, Bcv};
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_json_response, ok_ok_json_response};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, post, State};

/// *`GET /bcv`*
///
/// Typically mounted as **`/navigation/bcv`**
///
/// Returns the BCV cursor for the calling user on the
/// `default_language`. In Phase 2 (M5+), the language and user are
/// resolved from `LanguageContext`.
#[get("/bcv")]
pub async fn get_bcv(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
) -> status::Custom<(ContentType, String)> {
    let lang = state.default_language.clone();
    let bcv = match store.get_bcv(lang, COMPAT_USER).await {
        Ok(b) => b,
        Err(_) => state.bcv.lock().unwrap().clone(),
    };
    match serde_json::to_string(&bcv) {
        Ok(v) => ok_json_response(v),
        Err(e) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("Could not parse bcv state as JSON object: {}", e)),
        ),
    }
}

/// *`POST /bcv/<book_code>/<chapter>/<verse>`*
///
/// Typically mounted as **`/navigation/bcv/<book_code>/<chapter>/<verse>`**
///
/// Sets the BCV cursor for the calling user on the `default_language`.
/// Dual-write through the trait + AppSettings mirror so SSE
/// consumers see the change.
#[post("/bcv/<book_code>/<chapter>/<verse>")]
pub async fn post_bcv(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    book_code: &str,
    chapter: u16,
    verse: u16,
) -> status::Custom<(ContentType, String)> {
    let bcv = Bcv {
        book_code: book_code.to_string(),
        chapter,
        verse,
    };

    let lang = state.default_language.clone();
    if let Err(e) = store.put_bcv(lang, COMPAT_USER, bcv.clone()).await {
        return not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("Could not persist bcv: {}", e)),
        );
    }

    *state.bcv.lock().unwrap() = bcv;
    ok_ok_json_response()
}
