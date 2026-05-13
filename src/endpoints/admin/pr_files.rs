//! `GET /admin/pr-files?language=<code>&pr=<n>` — list files
//! changed in a PR plus the GitHub-rendered diff patches.

use crate::auth::{GithubAppAuth, GithubClient, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::endpoints::admin::context;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::{get, State};
use std::sync::Arc;

#[get("/pr-files?<language>&<pr>")]
#[allow(clippy::too_many_arguments)]
pub async fn list_pr_files(
    cookies: &CookieJar<'_>,
    catalog: &State<Arc<CatalogRegistry>>,
    app_auth: &State<Option<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    language: String,
    pr: u64,
) -> status::Custom<(ContentType, String)> {
    let ctx = match context::resolve(cookies, catalog, app_auth, tokens, github_client, &language)
        .await
    {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let files = match github_client
        .list_pull_files(&ctx.installation_token, &ctx.upstream, pr)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            return not_ok_json_response(
                Status::BadGateway,
                make_bad_json_data_response(format!("list pull files: {}", e)),
            );
        }
    };
    let body = serde_json::json!({
        "is_good": true,
        "language": language,
        "pr_number": pr,
        "files": files,
    });
    ok_json_response(body.to_string())
}
