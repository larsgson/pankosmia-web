//! GitHub webhook receiver for both the catalog repo and language
//! repos.
//!
//! Verifies the HMAC-SHA256 signature in `X-Hub-Signature-256`
//! against `GITHUB_WEBHOOK_SECRET`. Misconfigured / forged
//! requests return 401 with no body.
//!
//! On valid catalog webhook → re-pull `languages.yaml`, reload the
//! registry, log diff.
//!
//! On valid language webhook → `git fetch` the upstream cache for
//! that language, emit SSE change events via the existing
//! `WatcherRegistry` (the local clone's mtimes change → inotify
//! observed by the registry → broadcast to subscribers).

use rocket::data::{Data, ToByteUnit};
use rocket::http::{ContentType, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::status;
use rocket::{post, State};

use crate::catalog::{CatalogRegistry, SharedCatalogSync};
use crate::identity::LanguageCode;
use crate::store::SharedProjectStore;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use std::sync::Arc;

/// Signature header value, captured by a request guard.
pub struct GithubSignature(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for GithubSignature {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        match req.headers().get_one("X-Hub-Signature-256") {
            Some(s) => Outcome::Success(GithubSignature(s.to_string())),
            None => Outcome::Error((Status::Unauthorized, ())),
        }
    }
}

/// Verify HMAC-SHA256 (`sha256=...`) over the raw body using the
/// configured secret. Constant-time compare.
pub fn verify_signature(secret: &[u8], body: &[u8], header_value: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let prefix = "sha256=";
    let provided = match header_value.strip_prefix(prefix) {
        Some(s) => s,
        None => return false,
    };
    let provided_bytes = match hex_decode(provided) {
        Some(b) => b,
        None => return false,
    };
    let mut mac = match <Hmac<Sha256> as hmac::Mac>::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    mac.verify_slice(&provided_bytes).is_ok()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Configured webhook secret, managed as Rocket state.
pub struct WebhookSecret(pub Vec<u8>);

impl WebhookSecret {
    pub fn from_env() -> Self {
        let s = std::env::var("GITHUB_WEBHOOK_SECRET").unwrap_or_default();
        WebhookSecret(s.into_bytes())
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `POST /webhook/catalog`
///
/// Triggers reload of the catalog. The actual git fetch + YAML
/// reparse is done in a background task (so the webhook returns
/// 200 quickly and GitHub doesn't retry).
#[post("/webhook/catalog", data = "<body>")]
pub async fn catalog_webhook(
    sig: GithubSignature,
    secret: &State<WebhookSecret>,
    catalog: &State<Arc<CatalogRegistry>>,
    sync: &State<SharedCatalogSync>,
    body: Data<'_>,
) -> status::Custom<(ContentType, String)> {
    if secret.is_empty() {
        return not_ok_json_response(
            Status::ServiceUnavailable,
            make_bad_json_data_response("webhook secret not configured".into()),
        );
    }
    let bytes = match body.open(2.mebibytes()).into_bytes().await {
        Ok(b) if b.is_complete() => b.into_inner(),
        Ok(_) => {
            return not_ok_json_response(
                Status::PayloadTooLarge,
                make_bad_json_data_response("webhook payload too large".into()),
            );
        }
        Err(e) => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("read body: {}", e)),
            );
        }
    };
    if !verify_signature(&secret.0, &bytes, &sig.0) {
        return not_ok_json_response(
            Status::Unauthorized,
            make_bad_json_data_response("signature mismatch".into()),
        );
    }
    // Refresh the catalog in the background so we can return 200
    // quickly. git2 + file IO runs on the blocking pool.
    let catalog_clone = catalog.inner().clone();
    let sync_clone = sync.inner().clone();
    tokio::task::spawn_blocking(move || match sync_clone.refresh(&catalog_clone) {
        Ok(diff) => println!(
            "catalog webhook: reloaded ({} added, {} removed)",
            diff.added.len(),
            diff.removed.len()
        ),
        Err(e) => eprintln!("catalog webhook: refresh failed: {}", e),
    });
    ok_json_response(r#"{"is_good":true,"reason":"queued"}"#.into())
}

/// `POST /webhook/language/<code>`
///
/// Triggers `git fetch` of the language's upstream cache. SSE
/// subscribers see the resulting mtime changes via the
/// WatcherRegistry.
#[post("/webhook/language/<code>", data = "<body>")]
pub async fn language_webhook(
    sig: GithubSignature,
    secret: &State<WebhookSecret>,
    store: &State<SharedProjectStore>,
    code: &str,
    body: Data<'_>,
) -> status::Custom<(ContentType, String)> {
    if secret.is_empty() {
        return not_ok_json_response(
            Status::ServiceUnavailable,
            make_bad_json_data_response("webhook secret not configured".into()),
        );
    }
    let bytes = match body.open(2.mebibytes()).into_bytes().await {
        Ok(b) if b.is_complete() => b.into_inner(),
        Ok(_) => {
            return not_ok_json_response(
                Status::PayloadTooLarge,
                make_bad_json_data_response("webhook payload too large".into()),
            );
        }
        Err(e) => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("read body: {}", e)),
            );
        }
    };
    if !verify_signature(&secret.0, &bytes, &sig.0) {
        return not_ok_json_response(
            Status::Unauthorized,
            make_bad_json_data_response("signature mismatch".into()),
        );
    }
    let lang = match LanguageCode::parse(code) {
        Ok(l) => l,
        Err(_) => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("invalid language code: {}", code)),
            );
        }
    };
    // Kick off the fetch in the background so the webhook returns
    // quickly (GitHub retries on slow webhooks). The
    // WatcherRegistry will pick up the resulting file mtime changes
    // and broadcast SSE events to subscribers.
    let store_clone = store.inner().clone();
    tokio::spawn(async move {
        if let Err(e) = store_clone.prefetch_language(lang.clone()).await {
            eprintln!(
                "language webhook prefetch failed for {}: {}",
                lang.as_str(),
                e
            );
        }
    });
    ok_json_response(r#"{"is_good":true,"reason":"queued"}"#.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_verifies() {
        let secret = b"test-secret";
        let body = b"hello";
        // Pre-computed sha256 hmac of "hello" with key "test-secret":
        // openssl: echo -n "hello" | openssl dgst -sha256 -hmac "test-secret"
        let header = "sha256=bcc889a40667cab715e1dc22ad280692cf4bf1c3a280eeeca60d8dbcd8e4b993";
        assert!(verify_signature(secret, body, header));
    }

    #[test]
    fn signature_rejects_wrong() {
        assert!(!verify_signature(b"k", b"x", "sha256=00"));
        assert!(!verify_signature(b"k", b"x", "no prefix"));
        assert!(!verify_signature(b"k", b"x", "sha256=zz"));
    }
}
