//! `GET /admin/pending-prs?language=<code>` — list open PRs on a
//! language's upstream repo, filtered to pankosmia-origin edits.

use crate::auth::{GithubAppAuth, GithubClient, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::endpoints::admin::context;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::{get, State};
use std::sync::Arc;

const PANKOSMIA_BRANCH_PREFIX: &str = "pankosmia-edit-";

#[get("/pending-prs?<language>")]
#[allow(clippy::too_many_arguments)]
pub async fn list_pending_prs(
    cookies: &CookieJar<'_>,
    catalog: &State<Arc<CatalogRegistry>>,
    app_auth: &State<Option<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    language: String,
) -> status::Custom<(ContentType, String)> {
    let ctx = match context::resolve(
        cookies,
        catalog,
        app_auth,
        tokens,
        github_client,
        &language,
    )
    .await
    {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let prs = match github_client
        .list_pulls(&ctx.installation_token, &ctx.upstream, None, None, "open")
        .await
    {
        Ok(p) => p,
        Err(e) => {
            return not_ok_json_response(
                Status::BadGateway,
                make_bad_json_data_response(format!("list pulls: {}", e)),
            );
        }
    };
    // Filter to PRs whose head branch is a pankosmia-edit one. Other
    // PRs on the upstream (direct collaborators editing on GitHub)
    // are not surfaced here — they're a separate review track.
    let filtered: Vec<serde_json::Value> = prs
        .into_iter()
        .filter(|p| {
            p.head
                .as_ref()
                .map(|h| h.ref_.starts_with(PANKOSMIA_BRANCH_PREFIX))
                .unwrap_or(false)
        })
        .map(|p| {
            let head_login = p
                .head
                .as_ref()
                .and_then(|h| h.ref_.strip_prefix(PANKOSMIA_BRANCH_PREFIX))
                .unwrap_or("")
                .to_string();
            serde_json::json!({
                "pr_number": p.number,
                "pr_url": p.html_url,
                "title": p.title,
                "submitter_login": head_login,
                "created_at": p.created_at,
                "updated_at": p.updated_at,
            })
        })
        .collect();
    let body = serde_json::json!({
        "is_good": true,
        "language": language,
        "caller_login": ctx.login,
        "pending": filtered,
    });
    ok_json_response(body.to_string())
}
