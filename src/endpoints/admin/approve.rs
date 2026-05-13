//! `POST /admin/approve?language=<code>&pr=<n>&method=<squash|merge|rebase>` —
//! merge a PR via the GitHub App's installation token.

use crate::auth::{GithubAppAuth, GithubClient, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::endpoints::admin::context;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::{post, State};
use std::sync::Arc;

const DEFAULT_MERGE_METHOD: &str = "squash";
const ALLOWED_METHODS: &[&str] = &["squash", "merge", "rebase"];

#[post("/approve?<language>&<pr>&<method>")]
#[allow(clippy::too_many_arguments)]
pub async fn approve_pr(
    cookies: &CookieJar<'_>,
    catalog: &State<Arc<CatalogRegistry>>,
    app_auth: &State<Option<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    language: String,
    pr: u64,
    method: Option<String>,
) -> status::Custom<(ContentType, String)> {
    let ctx = match context::resolve(cookies, catalog, app_auth, tokens, github_client, &language)
        .await
    {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let merge_method = method.as_deref().unwrap_or(DEFAULT_MERGE_METHOD);
    if !ALLOWED_METHODS.contains(&merge_method) {
        return not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!(
                "merge method must be one of {:?}; got '{}'",
                ALLOWED_METHODS, merge_method
            )),
        );
    }
    match github_client
        .merge_pull_request(&ctx.installation_token, &ctx.upstream, pr, merge_method)
        .await
    {
        Ok(sha) => {
            let body = serde_json::json!({
                "is_good": true,
                "pr_number": pr,
                "merge_method": merge_method,
                "merge_sha": sha,
                "approver_login": ctx.login,
            });
            ok_json_response(body.to_string())
        }
        Err(e) => not_ok_json_response(
            Status::BadGateway,
            make_bad_json_data_response(format!("merge pr: {}", e)),
        ),
    }
}
