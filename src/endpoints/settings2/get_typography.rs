use crate::identity::LOCAL_USER;
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, State};

/// *`GET /typography`*
///
/// Typically mounted as **`/settings/typography`**
///
/// Returns the current typography settings.
#[get("/typography")]
pub(crate) async fn get_typography(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
) -> status::Custom<(ContentType, String)> {
    let typography = match store.get_typography(LOCAL_USER).await {
        Ok(t) => t,
        // Fall back to the AppSettings mirror until the trait has a
        // record (first POST will populate it).
        Err(_) => state.typography.lock().unwrap().clone(),
    };
    match serde_json::to_string(&typography) {
        Ok(v) => ok_json_response(v),
        Err(e) => not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!(
                "Could not parse typography settings as JSON object: {}",
                e
            )),
        ),
    }
}
