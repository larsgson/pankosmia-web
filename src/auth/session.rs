//! Session cookie helpers.
//!
//! Sessions in this design carry only an opaque user identifier
//! (the GitHub user-id, as a string). The actual OAuth token is
//! never sent to the browser; the server looks it up server-side
//! via `TokenStore` whenever it needs to call GitHub.
//!
//! Cookie attributes (set in `set_session`):
//!   - HttpOnly: yes
//!   - Secure: `true` when any configured origin uses HTTPS
//!     (`PANKOSMIA_PUBLIC_ORIGIN` or `PANKOSMIA_ALLOWED_ORIGINS`).
//!     Plain-HTTP local dev → not Secure (otherwise Safari and
//!     Firefox drop the cookie before the OAuth callback can read it).
//!   - SameSite: Lax — needs to be Lax (not Strict) so the OAuth
//!                callback redirect from github.com carries the
//!                cookie back. Strict would drop the cookie on
//!                the cross-site GET back from GitHub.
//!   - Path: /
//!   - Signed via Rocket's PrivateCookie support (configured via
//!     ROCKET_SECRET_KEY env in production).

use rocket::http::{Cookie, CookieJar, SameSite};

pub const SESSION_COOKIE_NAME: &str = "pankosmia_session";
pub const OAUTH_STATE_COOKIE_NAME: &str = "pankosmia_oauth_state";

fn use_secure_cookies() -> bool {
    if std::env::var("PANKOSMIA_PUBLIC_ORIGIN")
        .map(|o| o.starts_with("https://"))
        .unwrap_or(false)
    {
        return true;
    }
    std::env::var("PANKOSMIA_ALLOWED_ORIGINS")
        .map(|list| list.split(',').any(|e| e.trim().starts_with("https://")))
        .unwrap_or(false)
}

/// Set the session cookie identifying a logged-in user by their
/// GitHub user-id.
pub fn set_session(cookies: &CookieJar<'_>, github_user_id: i64) {
    let cookie = Cookie::build((SESSION_COOKIE_NAME, github_user_id.to_string()))
        .http_only(true)
        .secure(use_secure_cookies())
        .same_site(SameSite::Lax)
        .path("/")
        .build();
    cookies.add_private(cookie);
}

/// Retrieve the GitHub user-id from the session cookie, if any.
pub fn read_session(cookies: &CookieJar<'_>) -> Option<i64> {
    cookies
        .get_private(SESSION_COOKIE_NAME)
        .and_then(|c| c.value().parse::<i64>().ok())
}

/// Clear the session cookie (and any in-flight OAuth state).
pub fn clear_session(cookies: &CookieJar<'_>) {
    cookies.remove_private(SESSION_COOKIE_NAME);
    cookies.remove_private(OAUTH_STATE_COOKIE_NAME);
}

/// Set the CSRF state cookie used during the OAuth round-trip.
/// Cleared on callback success.
pub fn set_oauth_state(cookies: &CookieJar<'_>, state: &str) {
    let cookie = Cookie::build((OAUTH_STATE_COOKIE_NAME, state.to_string()))
        .http_only(true)
        .secure(use_secure_cookies())
        .same_site(SameSite::Lax)
        .path("/")
        .build();
    cookies.add_private(cookie);
}

pub fn read_oauth_state(cookies: &CookieJar<'_>) -> Option<String> {
    cookies
        .get_private(OAUTH_STATE_COOKIE_NAME)
        .map(|c| c.value().to_string())
}

pub fn clear_oauth_state(cookies: &CookieJar<'_>) {
    cookies.remove_private(OAUTH_STATE_COOKIE_NAME);
}
