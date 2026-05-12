use crate::identity::LOCAL_USER;
use crate::store::SharedProjectStore;
use crate::structs::{AppSettings, Typography};
use crate::utils::client::Clients;
use crate::utils::files::write_user_settings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_ok_json_response};
use crate::MsgQueue;
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{post, State};
use std::collections::BTreeMap;

/// *`POST /typography/<font_set>/<size>/<direction>`*
///
/// Typically mounted as **`/settings/typography/<font_set>/<size>/<direction>`**
///
/// Sets UI typography and interface direction. Dual-write through
/// the trait + AppSettings mirror + legacy `user_settings.json`.
#[post("/typography/<font_set>/<size>/<direction>")]
pub async fn post_typography(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    clients: &State<Clients>,
    msgs: &State<MsgQueue>,
    font_set: &str,
    size: &str,
    direction: &str,
) -> status::Custom<(ContentType, String)> {
    // Build the new Typography from existing features + new top-level fields.
    let new_typography = {
        let typo_inner = state.typography.lock().unwrap();
        let mut existing_features = BTreeMap::new();
        for (key, value) in &typo_inner.features {
            existing_features.insert(key.clone(), value.clone());
        }
        Typography {
            font_set: font_set.to_string(),
            size: size.to_string(),
            direction: direction.to_string(),
            features: existing_features,
        }
    };

    // Trait write — authoritative.
    if let Err(e) = store
        .put_typography(LOCAL_USER, new_typography.clone())
        .await
    {
        return not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("Could not persist typography: {}", e)),
        );
    }

    // Mirror update — SSE consumers see this.
    *state.typography.lock().unwrap() = new_typography;
    msgs.lock()
        .unwrap()
        .push_back("info--3--typography--change".to_string());

    if let Err(e) = write_user_settings(&state, &clients) {
        return not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!("Could not write out user settings: {}", e)),
        );
    }
    ok_ok_json_response()
}
