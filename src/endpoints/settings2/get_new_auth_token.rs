use crate::identity::COMPAT_USER;
use crate::store::SharedProjectStore;
use crate::structs::{AppSettings, ContentOrRedirect};
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::not_ok_json_response;
use rocket::http::{CookieJar, Status};
use rocket::response::Redirect;
use rocket::{get, State};

/// *`GET /auth-token/<token_key>/<code>/<client_code>`*
///
/// Typically mounted as **`/settings/auth-token/<token_key>/<code>/<client_code>`**
///
/// Landing URL for OAuth-style auth via a gateway server. Validates
/// the in-flight `AuthRequest` (one-shot) and stores the token. Both
/// trait + AppSettings mirror updated.
#[get("/auth-token/<token_key>/<code>/<client_code>")]
pub async fn get_new_auth_token(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    token_key: String,
    code: String,
    client_code: String,
    cj: &CookieJar<'_>,
) -> ContentOrRedirect {
    if !state.gitea_endpoints.contains_key(&token_key) {
        return ContentOrRedirect::Content(not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!("Unknown GITEA endpoint name: {}", token_key)),
        ));
    }

    // One-shot take of the in-flight auth request from the trait.
    let pending = match store.take_auth_request(COMPAT_USER, &token_key).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return ContentOrRedirect::Content(not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!(
                    "No record auth request found for {}",
                    token_key
                )),
            ));
        }
        Err(e) => {
            return ContentOrRedirect::Content(not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("Auth request lookup failed: {}", e)),
            ));
        }
    };

    if pending.code != client_code {
        return ContentOrRedirect::Content(not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!("Invalid client code for {}", token_key)),
        ));
    }
    let redirect_uri = format!("/{}", pending.redirect_uri);

    // Mirror: drop any in-memory copy too.
    state.auth_requests.lock().unwrap().remove(&token_key);

    if code.is_empty() {
        cj.remove(format!("{}_code", token_key));
        let _ = store.delete_auth_token(COMPAT_USER, &token_key).await;
        state.auth_tokens.lock().unwrap().remove(&token_key);
    } else {
        if let Err(e) = store.put_auth_token(COMPAT_USER, &token_key, &code).await {
            return ContentOrRedirect::Content(not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("Could not persist auth token: {}", e)),
            ));
        }
        state
            .auth_tokens
            .lock()
            .unwrap()
            .insert(token_key.clone(), code.clone());
        cj.add((format!("{}_code", token_key), code));
    }

    ContentOrRedirect::Redirect(Redirect::to(redirect_uri))
}
