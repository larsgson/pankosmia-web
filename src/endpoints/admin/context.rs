//! Shared auth + authorisation plumbing for `/admin/*` endpoints.
//!
//! Resolves: session → user identity → language code → installation
//! token → caller's permission on the upstream language repo.
//! Rejects with the appropriate HTTP status at the first failure.

use crate::auth::session::read_session;
use crate::auth::{resolve_installation_id, GithubAppAuth, GithubClient, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::identity::LanguageCode;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::not_ok_json_response;
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::State;
use std::sync::Arc;

/// Permissions sufficient to approve / reject PRs. `maintain` and
/// `admin` can merge; `write` / `triage` / `read` cannot.
pub const ADMIN_PERMISSIONS: &[&str] = &["admin", "maintain"];

/// Resolved context for an admin call.
pub struct AdminContext {
    pub login: String,
    pub _github_user_id: i64,
    pub upstream: String,
    pub installation_token: String,
}

/// Resolve auth + language + permission and return the context, or
/// a ready-to-return error response.
pub async fn resolve(
    cookies: &CookieJar<'_>,
    catalog: &State<Arc<CatalogRegistry>>,
    app_auth: &State<Option<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    language_code: &str,
) -> Result<AdminContext, status::Custom<(ContentType, String)>> {
    let github_user_id = read_session(cookies).ok_or_else(|| {
        not_ok_json_response(
            Status::Unauthorized,
            make_bad_json_data_response("not signed in".into()),
        )
    })?;
    let app_auth = app_auth.inner().as_ref().ok_or_else(|| {
        not_ok_json_response(
            Status::ServiceUnavailable,
            make_bad_json_data_response(
                "GitHub App auth not configured (GITHUB_APP_ID unset?)".into(),
            ),
        )
    })?;
    let lang = LanguageCode::parse(language_code).map_err(|_| {
        not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!("invalid language code: {}", language_code)),
        )
    })?;
    let entry = catalog.get(&lang).ok_or_else(|| {
        not_ok_json_response(
            Status::NotFound,
            make_bad_json_data_response(format!("language '{}' not in catalog", language_code)),
        )
    })?;
    let upstream = entry.repo.clone();

    let installation_id =
        resolve_installation_id(entry.installation_id, lang.as_str()).map_err(|e| {
            not_ok_json_response(
                Status::ServiceUnavailable,
                make_bad_json_data_response(format!("{}", e)),
            )
        })?;
    let installation_token = app_auth
        .installation_token(installation_id)
        .await
        .map_err(|e| {
            not_ok_json_response(
                Status::BadGateway,
                make_bad_json_data_response(format!("installation token: {}", e)),
            )
        })?;

    // Identity: fetch the user's login via their identity token.
    let user_token = tokens
        .load(github_user_id)
        .map_err(|e| {
            not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("token store: {}", e)),
            )
        })?
        .ok_or_else(|| {
            not_ok_json_response(
                Status::Unauthorized,
                make_bad_json_data_response("no stored token; please sign in again".into()),
            )
        })?;
    let user = github_client.get_user(&user_token).await.map_err(|e| {
        not_ok_json_response(
            Status::BadGateway,
            make_bad_json_data_response(format!("github /user: {}", e)),
        )
    })?;

    // Authorisation: caller must have admin/maintain permission on
    // the upstream repo. We use the App's installation token to ask
    // GitHub about the user's collaborator status (the App has
    // metadata:read on installed repos).
    let permission = github_client
        .get_repo_permission(&installation_token, &upstream, &user.login)
        .await
        .map_err(|e| {
            not_ok_json_response(
                Status::BadGateway,
                make_bad_json_data_response(format!("collaborator permission: {}", e)),
            )
        })?;
    let perm_str = permission
        .as_ref()
        .map(|p| p.permission.as_str())
        .unwrap_or("none");
    if !ADMIN_PERMISSIONS.contains(&perm_str) {
        return Err(not_ok_json_response(
            Status::Forbidden,
            make_bad_json_data_response(format!(
                "requires admin/maintain on {} (caller has '{}')",
                upstream, perm_str
            )),
        ));
    }

    Ok(AdminContext {
        login: user.login,
        _github_user_id: github_user_id,
        upstream,
        installation_token,
    })
}
