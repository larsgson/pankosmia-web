use crate::identity::COMPAT_USER;
use crate::static_vars::NET_IS_ENABLED;
use crate::store::{AuthRequest, SharedProjectStore};
use crate::structs::{AppSettings, AuthRequest as MirrorAuthRequest, ContentOrRedirect};
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, not_ok_offline_json_response};
use rocket::http::Status;
use rocket::response::Redirect;
use rocket::{get, State};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use uuid::Uuid;

/// *`GET /login/<auth_key>/<redir_path..>`*
///
/// Typically mounted as **`/gitea/login/<auth_key>/<redir_path..>`**
///
/// Initiates login to a remote server, redirecting to that server's
/// auth flow. Stores both an in-flight `AuthRequest` (consumed by
/// `get_new_auth_token` on callback) and clears any prior token.
#[get("/login/<token_key>/<redir_path..>")]
pub async fn gitea_proxy_login(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    token_key: String,
    redir_path: PathBuf,
) -> ContentOrRedirect {
    if !NET_IS_ENABLED.load(Ordering::Relaxed) {
        return ContentOrRedirect::Content(not_ok_offline_json_response());
    }
    if !state.gitea_endpoints.contains_key(&token_key) {
        return ContentOrRedirect::Content(not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!("Unknown GITEA endpoint name: {}", token_key)),
        ));
    }

    // Drop any existing token for this endpoint (trait + mirror).
    let _ = store.delete_auth_token(COMPAT_USER, &token_key).await;
    state.auth_tokens.lock().unwrap().remove(&token_key);

    // Record the in-flight auth request.
    let code = Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now();
    let req = AuthRequest {
        code: code.clone(),
        redirect_uri: redir_path.display().to_string(),
        timestamp: now,
    };
    if let Err(e) = store
        .put_auth_request(COMPAT_USER, &token_key, req.clone())
        .await
    {
        return ContentOrRedirect::Content(not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("Could not record auth request: {}", e)),
        ));
    }
    // Mirror — kept in sync for any code path still reading from it.
    let mut mirror = state.auth_requests.lock().unwrap();
    mirror.remove(&token_key);
    mirror.insert(
        token_key.clone(),
        MirrorAuthRequest {
            code: code.clone(),
            redirect_uri: req.redirect_uri,
            timestamp: now,
        },
    );
    drop(mirror);

    ContentOrRedirect::Redirect(Redirect::to(format!(
        "{}/auth?client_code={}&redir_path=%2F",
        state.gitea_endpoints[&token_key].clone(),
        &code
    )))
}
