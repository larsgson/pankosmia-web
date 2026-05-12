//! `POST /admin/reject?language=<code>&pr=<n>&reason=<text>` —
//! close a PR without merging, optionally with a comment.

use crate::auth::{GithubAppAuth, GithubClient, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::endpoints::admin::context;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::{post, State};
use std::sync::Arc;

#[post("/reject?<language>&<pr>&<reason>")]
#[allow(clippy::too_many_arguments)]
pub async fn reject_pr(
    cookies: &CookieJar<'_>,
    catalog: &State<Arc<CatalogRegistry>>,
    app_auth: &State<Option<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    language: String,
    pr: u64,
    reason: Option<String>,
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
    if let Some(reason_text) = reason.as_ref() {
        if !reason_text.is_empty() {
            let comment = format!(
                "Rejected by @{}:\n\n{}",
                ctx.login,
                reason_text
            );
            if let Err(e) = github_client
                .add_pr_comment(&ctx.installation_token, &ctx.upstream, pr, &comment)
                .await
            {
                return not_ok_json_response(
                    Status::BadGateway,
                    make_bad_json_data_response(format!("add comment: {}", e)),
                );
            }
        }
    }
    match github_client
        .close_pull_request(&ctx.installation_token, &ctx.upstream, pr)
        .await
    {
        Ok(()) => {
            let body = serde_json::json!({
                "is_good": true,
                "pr_number": pr,
                "closed_by_login": ctx.login,
                "reason_recorded": reason.is_some(),
            });
            ok_json_response(body.to_string())
        }
        Err(e) => not_ok_json_response(
            Status::BadGateway,
            make_bad_json_data_response(format!("close pr: {}", e)),
        ),
    }
}
