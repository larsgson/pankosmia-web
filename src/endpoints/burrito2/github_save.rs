//! Shared helpers for save-style endpoints under the GitHub backend.
//!
//! Each burrito2 save endpoint has its own URL shape and FS branch,
//! but the GitHub-backed branch always needs the same upfront
//! plumbing: read the session cookie, resolve the X-Language-Code
//! header, load the user's identity token, call `GET /user` for the
//! login, then dispatch into `GithubEditFlow::apply_op` with a
//! variant-specific `SaveOp`. This module hosts that pipeline so
//! each endpoint stays a thin wrapper.

use crate::auth::session::read_session;
use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::server::{LanguageLocks, RateLimitError, RateLimiter};
use crate::store::github::audio_ref::{
    head_validate_url, is_audio_ref_path, validate_schema, AudioRefConfig,
};
use crate::store::github::{GithubEditFlow, SaveOp, SaveOutcome};
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::check_path_string_components;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::State;
use std::env;

/// Maximum raw bytes per ingredient PUT/upload via the App flow.
/// The GitHub Contents API caps requests at ~1 MB (base64-encoded),
/// which is ~750 KB raw. We keep some headroom for the JSON envelope
/// + commit message.
pub const MAX_INGREDIENT_BYTES: usize = 700_000;

pub fn is_github_backend() -> bool {
    env::var("STORAGE_BACKEND")
        .map(|v| v.eq_ignore_ascii_case("github"))
        .unwrap_or(false)
}

/// Endpoint contract: validate one or more ingredient-path strings
/// before dispatching. Returns `Ok(())` if all are well-formed,
/// otherwise a ready-to-return 400 response.
pub fn validate_ipath_segments(
    ipaths: &[&str],
) -> Result<(), status::Custom<(ContentType, String)>> {
    for p in ipaths {
        if !check_path_string_components(p.to_string()) {
            return Err(not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("bad ingredient path: {}", p)),
            ));
        }
    }
    Ok(())
}

/// 501 reply for save endpoints whose multi-file shape requires the
/// Git Data API (atomic commits across many files) and is not yet
/// implemented under the GitHub backend.
pub fn not_implemented_github(op: &str) -> status::Custom<(ContentType, String)> {
    not_ok_json_response(
        Status::NotImplemented,
        make_bad_json_data_response(format!(
            "{} is not yet implemented for STORAGE_BACKEND=github (requires Git Data API)",
            op
        )),
    )
}

/// Run a `SaveOp` against the GitHub edit flow. Encapsulates the
/// session-cookie → rate-limit → size-check → token → login →
/// apply_op pipeline used by every single-file save endpoint.
#[allow(clippy::too_many_arguments)]
pub async fn handle_github_op<'a>(
    cookies: &CookieJar<'_>,
    edit_flow: &State<GithubEditFlow>,
    app_auth: &State<Option<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    locks: &State<LanguageLocks>,
    rate_limiter: &State<RateLimiter>,
    audio_ref_cfg: &State<AudioRefConfig>,
    language_header: Option<LanguageHeader>,
    op: SaveOp<'a>,
    commit_message: &str,
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
    // Rate limit BEFORE any GitHub-touching work. Saves both our App
    // installation-token quota and the attacker's appetite if the
    // 429 is fast and cheap to receive.
    if let Err(RateLimitError::Exceeded(retry_after)) = rate_limiter.check(github_user_id) {
        return not_ok_json_response(
            Status::TooManyRequests,
            make_bad_json_data_response(format!(
                "rate limit exceeded; retry in {}s",
                retry_after
            )),
        );
    }
    // Size cap for any bytes the user is writing. Reverts/copies
    // read from upstream so they're naturally bounded by what's
    // already on GitHub.
    if let SaveOp::Put { bytes, .. } = &op {
        if bytes.len() > MAX_INGREDIENT_BYTES {
            return not_ok_json_response(
                Status::PayloadTooLarge,
                make_bad_json_data_response(format!(
                    "ingredient too large: {} bytes (max {} via Contents API)",
                    bytes.len(),
                    MAX_INGREDIENT_BYTES
                )),
            );
        }
    }
    // Audio-reference validation: writes to `audio_content/**/ref.json`
    // (or `*.audioref`) are validated against the v1 schema + license
    // allowlist before any GitHub round-trip. Audio bytes themselves
    // live elsewhere (Internet Archive); the burrito only stores small
    // reference JSON files. See `docs/impl/AUDIO_STRATEGY.md`.
    if let SaveOp::Put { ipath, bytes } = &op {
        if is_audio_ref_path(ipath) {
            if let Err(e) = validate_schema(bytes, audio_ref_cfg.inner()) {
                return not_ok_json_response(
                    Status::BadRequest,
                    make_bad_json_data_response(format!("audio reference: {}", e)),
                );
            }
            // Optional HEAD-reachability check. Off by default; opt-in
            // via PANKOSMIA_VALIDATE_AUDIO_URLS=true.
            if audio_ref_cfg.validate_urls {
                if let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(bytes) {
                    let urls = extract_urls_for_head(&parsed);
                    for url in urls {
                        match head_validate_url(&url).await {
                            Ok(true) => {}
                            Ok(false) => {
                                return not_ok_json_response(
                                    Status::BadRequest,
                                    make_bad_json_data_response(format!(
                                        "audio reference URL {} returned non-audio Content-Type",
                                        url
                                    )),
                                );
                            }
                            // HEAD failed / timed out — accept the
                            // write but flag it. Bytes will be visible
                            // in the response header.
                            Err(_) => {
                                // Continue silently; the write
                                // succeeds. Client can be told via a
                                // separate signal if needed later.
                            }
                        }
                    }
                }
            }
        }
    }
    let lang = match language_header {
        Some(LanguageHeader(l)) => l,
        None => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(
                    "X-Language-Code header required for GitHub backend".into(),
                ),
            );
        }
    };
    let app_auth = match app_auth.inner().as_ref() {
        Some(a) => a,
        None => {
            return not_ok_json_response(
                Status::ServiceUnavailable,
                make_bad_json_data_response(
                    "GitHub App auth not configured (GITHUB_APP_ID unset?)".into(),
                ),
            );
        }
    };
    let token = match tokens.load(github_user_id) {
        Ok(Some(t)) => t,
        Ok(None) => {
            return not_ok_json_response(
                Status::Unauthorized,
                make_bad_json_data_response("no stored token; please sign in again".into()),
            );
        }
        Err(e) => {
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("token store: {}", e)),
            );
        }
    };
    let user = match github_client.get_user(&token).await {
        Ok(u) => u,
        Err(e) => {
            return not_ok_json_response(
                Status::BadGateway,
                make_bad_json_data_response(format!("github /user: {}", e)),
            );
        }
    };
    match edit_flow
        .apply_op(
            &user.login,
            github_user_id,
            lang,
            op,
            commit_message,
            github_client.inner(),
            app_auth,
            locks.inner(),
        )
        .await
    {
        Ok(outcome) => ok_save_outcome_response(&outcome),
        Err(e) => not_ok_json_response(
            Status::BadGateway,
            make_bad_json_data_response(format!("github edit flow: {}", e)),
        ),
    }
}

/// Pull the audio URL(s) out of a validated reference JSON. Handles
/// both flat and multi-take shapes. Assumes prior schema validation
/// passed (i.e. `url` strings are present and shaped sensibly).
fn extract_urls_for_head(v: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(u) = v.get("url").and_then(|x| x.as_str()) {
        out.push(u.to_string());
    }
    if let Some(takes) = v.get("takes").and_then(|x| x.as_array()) {
        for t in takes {
            if let Some(u) = t.get("url").and_then(|x| x.as_str()) {
                out.push(u.to_string());
            }
        }
    }
    out
}

fn ok_save_outcome_response(outcome: &SaveOutcome) -> status::Custom<(ContentType, String)> {
    let body = serde_json::json!({
        "is_good": true,
        "status": outcome.status,
        "branch": outcome.branch,
        "pr_url": outcome.pr_url,
        "pr_number": outcome.pr_number,
    });
    ok_json_response(body.to_string())
}
