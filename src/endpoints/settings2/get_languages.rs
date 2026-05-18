use crate::identity::COMPAT_USER;
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, State};

/// *`GET /languages`*
///
/// Typically mounted as **`/settings/languages`**
///
/// Returns an array containing the current selected UI languages.
///
/// `["en"]`
///
/// Legacy compatibility endpoint. Real per-user language management
/// goes through `/user-languages/`.
#[get("/languages")]
pub async fn get_languages(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
) -> status::Custom<(ContentType, String)> {
    let langs: Vec<String> = match store.get_languages(COMPAT_USER).await {
        Ok(ls) if !ls.is_empty() => ls.iter().map(|l| l.to_string()).collect(),
        // NotFound or empty: fall back to AppSettings mirror.
        _ => state.languages.lock().unwrap().clone(),
    };
    match serde_json::to_string(&langs) {
        Ok(v) => ok_json_response(v),
        Err(e) => not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!(
                "Could not parse language settings as JSON array: {}",
                e
            )),
        ),
    }
}
