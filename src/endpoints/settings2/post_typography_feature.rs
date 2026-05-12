use crate::identity::LOCAL_USER;
use crate::store::SharedProjectStore;
use crate::structs::{AppSettings, Typography, TypographyFeature};
use crate::utils::client::Clients;
use crate::utils::files::{copy_and_customize_webfont_css2, write_user_settings};
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{source_webfonts_path, webfonts_path};
use crate::utils::response::{not_ok_json_response, ok_ok_json_response};
use crate::MsgQueue;
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{post, State};
use std::collections::BTreeMap;

/// *`POST /typography-feature/<font_name>/<feature>/<value>`*
///
/// Typically mounted as **`/settings/typography-feature/<font_name>/<feature>/<value>`**
///
/// Sets the value of a font feature. Currently silently ignores
/// unknown fonts and fields. Dual-write through the trait +
/// AppSettings mirror + legacy `user_settings.json`.
#[post("/typography-feature/<font_name>/<feature>/<new_value>")]
pub async fn post_typography_feature(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    clients: &State<Clients>,
    msgs: &State<MsgQueue>,
    font_name: &str,
    feature: &str,
    new_value: u8,
) -> status::Custom<(ContentType, String)> {
    let working_dir = state.working_dir.clone();
    let app_resources_dir = state.app_resources_dir.clone();
    let src_webfonts_dir = source_webfonts_path(&app_resources_dir);
    let target_webfonts_dir = webfonts_path(&working_dir);

    // Compute the new Typography while holding the mirror lock.
    let new_typography = {
        let typo_inner = state.typography.lock().unwrap();
        let mut new_font_fields: BTreeMap<String, Vec<TypographyFeature>> = BTreeMap::new();
        for (font_key, font_value) in &typo_inner.features {
            if font_key == font_name {
                let mut new_fields: Vec<TypographyFeature> = Vec::new();
                for field_kv in font_value {
                    let value = if field_kv.key == feature {
                        new_value
                    } else {
                        field_kv.value
                    };
                    new_fields.push(TypographyFeature {
                        key: field_kv.key.clone(),
                        value,
                    });
                }
                if let Err(e) = copy_and_customize_webfont_css2(
                    &src_webfonts_dir,
                    &target_webfonts_dir,
                    &new_fields,
                    &font_name.to_string(),
                ) {
                    return not_ok_json_response(
                        Status::BadRequest,
                        make_bad_json_data_response(format!("Could not rewrite CSS: {}", e)),
                    );
                }
                new_font_fields.insert(font_key.clone(), new_fields);
            } else {
                new_font_fields.insert(font_key.clone(), font_value.clone());
            }
        }
        Typography {
            font_set: typo_inner.font_set.clone(),
            size: typo_inner.size.clone(),
            direction: typo_inner.direction.clone(),
            features: new_font_fields,
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

    // Mirror update.
    *state.typography.lock().unwrap() = new_typography;
    msgs.lock()
        .unwrap()
        .push_back("info--3--typography-feature--change".to_string());

    if let Err(e) = write_user_settings(&state, &clients) {
        return not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("Could not write out user settings: {}", e)),
        );
    }
    ok_ok_json_response()
}
