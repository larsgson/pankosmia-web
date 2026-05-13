//! OAuth flow + session endpoints.
//!
//! Endpoints:
//!   * `GET /auth/start?redirect=...` — redirects to github.com with
//!     a CSRF state cookie.
//!   * `GET /auth/callback?code=...&state=...` — finishes OAuth,
//!     persists the encrypted token, sets the session cookie,
//!     redirects back to the original page.
//!   * `POST /auth/logout` — clears the session cookie.
//!   * `GET /me` — returns the current user's profile (cached,
//!     refreshed lazily via `GET /user`).
//!
//! GitHub is the only identity provider; the URLs are deliberately
//! not provider-namespaced.

use crate::auth::github_client::GithubClient;
use crate::auth::session::{
    clear_oauth_state, clear_session, read_oauth_state, read_session, set_oauth_state, set_session,
};
use crate::auth::token_store::TokenStore;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::response::Redirect;
use rocket::{get, post, State};
use serde::Serialize;
use uuid::Uuid;

// No `scope` query parameter is sent. The GitHub App's user-to-server
// flow inherits scopes from the App's declared permissions; the
// classic OAuth `scope=` parameter is ignored. Identity-only login.

fn server_origin() -> String {
    std::env::var("PANKOSMIA_PUBLIC_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:19119".into())
}

fn callback_url() -> String {
    format!("{}/auth/callback", server_origin())
}

/// `GET /auth/start?redirect=/some/path`
///
/// Generates a CSRF state, stashes it in a cookie, redirects the
/// browser to GitHub's OAuth authorize endpoint. After approval,
/// GitHub bounces back to `/auth/callback`.
#[get("/auth/start?<redirect>")]
pub fn auth_github_start(
    cookies: &CookieJar<'_>,
    client: &State<GithubClient>,
    redirect: Option<String>,
) -> Redirect {
    let state = format!(
        "{}|{}",
        Uuid::new_v4(),
        urlencoding::encode(redirect.as_deref().unwrap_or("/"))
    );
    set_oauth_state(cookies, &state);
    let url = format!(
        "https://github.com/login/oauth/authorize\
         ?client_id={}&redirect_uri={}&state={}",
        urlencoding::encode(&client.client_id),
        urlencoding::encode(&callback_url()),
        urlencoding::encode(&state),
    );
    Redirect::to(url)
}

/// `GET /auth/callback?code=...&state=...`
///
/// Validates state, exchanges code for token, persists, sets
/// session, redirects to the original page.
#[get("/auth/callback?<code>&<state>")]
pub async fn auth_github_callback(
    cookies: &CookieJar<'_>,
    client: &State<GithubClient>,
    tokens: &State<TokenStore>,
    code: Option<String>,
    state: Option<String>,
) -> Result<Redirect, status::Custom<(ContentType, String)>> {
    let code = code.ok_or_else(|| {
        not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response("missing code".into()),
        )
    })?;
    let state = state.ok_or_else(|| {
        not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response("missing state".into()),
        )
    })?;
    let expected = read_oauth_state(cookies).ok_or_else(|| {
        not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response("no oauth state cookie".into()),
        )
    })?;
    if expected != state {
        return Err(not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response("oauth state mismatch".into()),
        ));
    }
    clear_oauth_state(cookies);

    let token = client
        .exchange_oauth_code(&code, &callback_url())
        .await
        .map_err(|e| {
            not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("oauth exchange: {}", e)),
            )
        })?;
    let user = client.get_user(&token.access_token).await.map_err(|e| {
        not_ok_json_response(
            Status::BadGateway,
            make_bad_json_data_response(format!("github /user: {}", e)),
        )
    })?;
    tokens.save(user.id, &token.access_token).map_err(|e| {
        not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("token store: {}", e)),
        )
    })?;
    set_session(cookies, user.id);

    // Decode the original redirect path from the state.
    let redirect_to = state
        .split_once('|')
        .map(|(_, r)| urlencoding::decode(r).unwrap_or_default().into_owned())
        .filter(|s| s.starts_with('/') && !s.starts_with("//"))
        .unwrap_or_else(|| "/".into());
    Ok(Redirect::to(redirect_to))
}

/// `POST /auth/logout` — clear session.
#[post("/auth/logout")]
pub fn auth_logout(cookies: &CookieJar<'_>) -> status::Custom<(ContentType, String)> {
    clear_session(cookies);
    ok_json_response(r#"{"is_good":true}"#.into())
}

#[derive(Serialize)]
struct MeResponse {
    github_user_id: i64,
    login: String,
    name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
}

/// `GET /me`
///
/// Returns the calling user's GitHub profile. Calls `GET /user` on
/// every request for now; can be cached aggressively later (the
/// data only changes when the user updates their GitHub profile).
#[get("/me")]
pub async fn me(
    cookies: &CookieJar<'_>,
    client: &State<GithubClient>,
    tokens: &State<TokenStore>,
) -> status::Custom<(ContentType, String)> {
    let github_user_id = match read_session(cookies) {
        Some(id) => id,
        None => {
            return not_ok_json_response(
                Status::Unauthorized,
                make_bad_json_data_response("not signed in".into()),
            );
        }
    };
    let token = match tokens.load(github_user_id) {
        Ok(Some(t)) => t,
        Ok(None) => {
            clear_session(cookies);
            return not_ok_json_response(
                Status::Unauthorized,
                make_bad_json_data_response("token missing; please sign in again".into()),
            );
        }
        Err(e) => {
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("token store: {}", e)),
            );
        }
    };
    match client.get_user(&token).await {
        Ok(u) => {
            let body = MeResponse {
                github_user_id: u.id,
                login: u.login,
                name: u.name,
                email: u.email,
                avatar_url: u.avatar_url,
            };
            ok_json_response(serde_json::to_string(&body).unwrap_or_else(|_| "{}".into()))
        }
        Err(e) => {
            // Token revoked → drop the session so the client
            // re-signs in.
            clear_session(cookies);
            not_ok_json_response(
                Status::Unauthorized,
                make_bad_json_data_response(format!("github /user: {}", e)),
            )
        }
    }
}
