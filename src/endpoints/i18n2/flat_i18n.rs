use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::os_slash_str;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, State};
use serde_json::{Map, Value};
use std::path::PathBuf;

/// *`GET /flat/<filter>`*
///
/// Typically mounted as **`/i18n/flat/<filter>`**
///
/// Returns a flat object containing each i18n key with the best match based on the language preference settings. The optional filter restricts the keys returned. So, for `/i18n/flat/flavors` the response might be
///
/// ```text
/// {
///   "flavors:names:parascriptural/x-bcvArticles": "Articles by Verse",
///   "flavors:names:parascriptural/x-bcvImages": "Images by Verse",
///   "flavors:names:parascriptural/x-bcvNotes": "Notes by Verse",
///   "flavors:names:parascriptural/x-videolinks": "Video Links",
///   "flavors:names:scripture/textTranslation": "Scripture (Text)"
/// }
/// ```
#[get("/flat/<filter..>")]
pub async fn flat_i18n(
    state: &State<AppSettings>,
    filter: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let path_to_serve = state.working_dir.clone() + os_slash_str() + "i18n.json";
    let filter_items: Vec<String> = filter
        .display()
        .to_string()
        .split('/')
        .map(String::from)
        .collect();
    if filter_items.len() > 2 {
        return not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(
                format!("expected 0 - 2 filter terms, not {}", filter_items.len()).to_string(),
            ),
        );
    }
    let mut type_filter: Option<String> = None;
    let mut subtype_filter: Option<String> = None;
    if filter_items.len() > 0 && filter_items[0] != "" {
        type_filter = Some(filter_items[0].clone());
        if filter_items.len() > 1 && filter_items[1] != "" {
            subtype_filter = Some(filter_items[1].clone());
        }
    }
    let file_content = match std::fs::read_to_string(path_to_serve) {
        Ok(v) => v,
        Err(e) => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("could not read for flat i18n: {}", e)),
            );
        }
    };
    let sj = match serde_json::from_str::<Value>(file_content.as_str()) {
        Ok(v) => v,
        Err(e) => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("could not parse for flat i18n: {}", e)),
            );
        }
    };
    let top = match sj.as_object() {
        Some(v) => v,
        None => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response("i18n.json top level is not an object".to_string()),
            );
        }
    };
    let languages = match state.languages.lock() {
        Ok(v) => v.clone(),
        Err(e) => {
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("could not lock languages: {}", e)),
            );
        }
    };
    let mut flat = Map::new();
    for (i18n_type, subtypes) in top {
        if let Some(ref v) = type_filter {
            if v != i18n_type {
                continue;
            }
        }
        let subtypes_obj = match subtypes.as_object() {
            Some(v) => v,
            None => continue,
        };
        for (i18n_subtype, terms) in subtypes_obj {
            if let Some(ref v) = subtype_filter {
                if v != i18n_subtype {
                    continue;
                }
            }
            let terms_obj = match terms.as_object() {
                Some(v) => v,
                None => continue,
            };
            for (i18n_term, term_languages) in terms_obj {
                let lang_obj = match term_languages.as_object() {
                    Some(v) => v,
                    None => continue,
                };
                'user_lang: for user_language in &languages {
                    for (i18n_language, translation) in lang_obj {
                        if i18n_language == user_language {
                            let flat_key = format!(
                                "{}:{}:{}",
                                i18n_type, i18n_subtype, i18n_term
                            );
                            flat.insert(flat_key, translation.clone());
                            break 'user_lang;
                        }
                    }
                }
            }
        }
    }
    ok_json_response(serde_json::to_string(&flat).unwrap())
}
