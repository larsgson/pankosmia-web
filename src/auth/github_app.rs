//! GitHub App authentication.
//!
//! Two distinct token types live here:
//!
//!   1. **App JWT** — short-lived (≤10 min) RS256-signed JWT, used to
//!      authenticate as the App itself when calling `/app/...` endpoints
//!      (notably `POST /app/installations/<id>/access_tokens`). Minted
//!      on demand; never cached (cheap to make, hard to share safely).
//!   2. **Installation token** — short-lived (~1 h) bearer token,
//!      scoped to one installation's permissions on its installed
//!      repos. Used by the edit-flow code path to write to upstream
//!      language repos. Cached per `installation_id` until it nears
//!      expiry.
//!
//! The App's private key is loaded once at startup (via either
//! `GITHUB_APP_PRIVATE_KEY_PATH` pointing at a `.pem` file, or
//! `GITHUB_APP_PRIVATE_KEY` containing the PEM contents directly —
//! useful for PaaS deployments that inject secrets as env vars).
//!
//! Design rationale: see `internal-docs/AUTH_MODEL.md`.

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Refresh installation tokens this many seconds before GitHub-side
/// expiry so we never hand out a token about to expire mid-request.
const INSTALLATION_TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);
/// JWT lifetime. GitHub allows ≤10 min; we use 9 min to give clock
/// skew breathing room.
const APP_JWT_LIFETIME: Duration = Duration::from_secs(9 * 60);
/// GitHub recommends issuing the JWT with `iat` 60 s in the past to
/// tolerate the App server's clock running ahead of GitHub's.
const APP_JWT_IAT_BACKDATE: Duration = Duration::from_secs(60);

const GITHUB_API: &str = "https://api.github.com";
const USER_AGENT: &str = "pankosmia-docker";

#[derive(Debug, thiserror::Error)]
pub enum GithubAppError {
    #[error("private key: {0}")]
    Key(String),
    #[error("sign jwt: {0}")]
    Sign(String),
    #[error("network: {0}")]
    Network(String),
    #[error("github api {status}: {body}")]
    Api { status: u16, body: String },
    #[error("decode: {0}")]
    Decode(String),
    #[error("clock: {0}")]
    Clock(String),
    #[error(
        "no installation id configured for language '{0}' and no \
             PANKOSMIA_DEFAULT_INSTALLATION_ID env var set"
    )]
    NoInstallationId(String),
}

/// JWT claims for App authentication. GitHub doesn't check anything
/// beyond `iat`/`exp`/`iss`; `aud`/`sub` are not required.
#[derive(Serialize)]
struct AppClaims {
    iat: u64,
    exp: u64,
    iss: String,
}

/// App-level credentials and a per-installation token cache.
pub struct GithubAppAuth {
    app_id: u64,
    encoding_key: EncodingKey,
    client: reqwest::Client,
    cache: Mutex<HashMap<u64, CachedToken>>,
}

#[derive(Clone)]
struct CachedToken {
    token: String,
    /// Absolute time at which the token expires per GitHub's response.
    expires_at: SystemTime,
}

impl CachedToken {
    fn is_fresh(&self, now: SystemTime) -> bool {
        match self.expires_at.duration_since(now) {
            Ok(remaining) => remaining > INSTALLATION_TOKEN_REFRESH_MARGIN,
            Err(_) => false,
        }
    }
}

#[derive(Deserialize)]
struct InstallationTokenResponse {
    token: String,
    expires_at: String,
}

impl GithubAppAuth {
    /// Construct from the App ID and PEM-encoded RSA private key
    /// bytes. The key bytes should contain the full PEM including
    /// the BEGIN/END lines.
    pub fn new(app_id: u64, private_key_pem: &[u8]) -> Result<Self, GithubAppError> {
        let encoding_key = EncodingKey::from_rsa_pem(private_key_pem)
            .map_err(|e| GithubAppError::Key(e.to_string()))?;
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| GithubAppError::Network(e.to_string()))?;
        Ok(Self {
            app_id,
            encoding_key,
            client,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Build from environment. Tries `GITHUB_APP_PRIVATE_KEY_PATH`
    /// first (preferred for local dev); falls back to
    /// `GITHUB_APP_PRIVATE_KEY` as inline PEM (PaaS-friendly). The
    /// `GITHUB_APP_ID` env var is required either way.
    ///
    /// Returns `Ok(None)` when no App credentials are configured at
    /// all.
    pub fn from_env() -> Result<Option<Self>, GithubAppError> {
        let app_id = match std::env::var("GITHUB_APP_ID") {
            Ok(s) if !s.is_empty() => s
                .parse::<u64>()
                .map_err(|e| GithubAppError::Key(format!("GITHUB_APP_ID not numeric: {}", e)))?,
            _ => return Ok(None),
        };
        let pem_bytes = match std::env::var("GITHUB_APP_PRIVATE_KEY_PATH") {
            Ok(p) if !p.is_empty() => {
                std::fs::read(&p).map_err(|e| GithubAppError::Key(format!("read {}: {}", p, e)))?
            }
            _ => match std::env::var("GITHUB_APP_PRIVATE_KEY") {
                Ok(s) if !s.is_empty() => s.into_bytes(),
                _ => {
                    return Err(GithubAppError::Key(
                        "GITHUB_APP_ID set but neither GITHUB_APP_PRIVATE_KEY_PATH \
                         nor GITHUB_APP_PRIVATE_KEY is set"
                            .into(),
                    ))
                }
            },
        };
        Self::new(app_id, &pem_bytes).map(Some)
    }

    /// Mint a fresh App JWT. Cheap (a single RSA sign), so we do not
    /// cache it.
    pub fn mint_app_jwt(&self) -> Result<String, GithubAppError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| GithubAppError::Clock(e.to_string()))?;
        let iat = now.saturating_sub(APP_JWT_IAT_BACKDATE).as_secs();
        let exp = (now + APP_JWT_LIFETIME).as_secs();
        let claims = AppClaims {
            iat,
            exp,
            iss: self.app_id.to_string(),
        };
        encode(&Header::new(Algorithm::RS256), &claims, &self.encoding_key)
            .map_err(|e| GithubAppError::Sign(e.to_string()))
    }

    /// Get a usable installation token for `installation_id`. Returns
    /// a cached one if still fresh; otherwise calls GitHub to mint a
    /// new one.
    pub async fn installation_token(&self, installation_id: u64) -> Result<String, GithubAppError> {
        let now = SystemTime::now();
        if let Some(cached) = self.cache.lock().get(&installation_id).cloned() {
            if cached.is_fresh(now) {
                return Ok(cached.token);
            }
        }
        let jwt = self.mint_app_jwt()?;
        let url = format!(
            "{}/app/installations/{}/access_tokens",
            GITHUB_API, installation_id
        );
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&jwt)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| GithubAppError::Network(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GithubAppError::Api {
                status: status.as_u16(),
                body,
            });
        }
        let body: InstallationTokenResponse = resp
            .json()
            .await
            .map_err(|e| GithubAppError::Decode(e.to_string()))?;
        // RFC3339 → SystemTime via chrono (already a dep).
        let expires_at = chrono::DateTime::parse_from_rfc3339(&body.expires_at)
            .map_err(|e| GithubAppError::Decode(format!("expires_at: {}", e)))?
            .with_timezone(&chrono::Utc);
        let expires_at =
            SystemTime::UNIX_EPOCH + Duration::from_secs(expires_at.timestamp() as u64);
        let cached = CachedToken {
            token: body.token.clone(),
            expires_at,
        };
        self.cache.lock().insert(installation_id, cached);
        Ok(body.token)
    }
}

/// Resolve the installation ID for a language. Per-language override
/// takes precedence; otherwise the global default from
/// `PANKOSMIA_DEFAULT_INSTALLATION_ID` is used.
pub fn resolve_installation_id(
    lang_override: Option<u64>,
    lang_code_for_error: &str,
) -> Result<u64, GithubAppError> {
    if let Some(id) = lang_override {
        return Ok(id);
    }
    match std::env::var("PANKOSMIA_DEFAULT_INSTALLATION_ID") {
        Ok(s) if !s.is_empty() => s.parse::<u64>().map_err(|e| {
            GithubAppError::Key(format!(
                "PANKOSMIA_DEFAULT_INSTALLATION_ID not numeric: {}",
                e
            ))
        }),
        _ => Err(GithubAppError::NoInstallationId(lang_code_for_error.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{decode, DecodingKey, Validation};

    /// Generate a fresh RSA keypair at test time via the `openssl`
    /// CLI. Avoids embedding a fixed test key (which would be a
    /// maintenance and review-time hazard even though it has no
    /// security value). Requires `openssl` on `PATH` — present on
    /// macOS, every common Linux distro, and standard CI images.
    fn generate_test_rsa_keypair() -> (String, String) {
        use std::process::{Command, Stdio};
        let priv_out = Command::new("openssl")
            .args(["genrsa", "2048"])
            .stderr(Stdio::null())
            .output()
            .expect("openssl on PATH");
        assert!(priv_out.status.success(), "openssl genrsa failed");
        let priv_pem = String::from_utf8(priv_out.stdout).unwrap();

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &priv_pem).unwrap();
        let pub_out = Command::new("openssl")
            .args(["rsa", "-in", tmp.path().to_str().unwrap(), "-pubout"])
            .stderr(Stdio::null())
            .output()
            .expect("openssl on PATH");
        assert!(pub_out.status.success(), "openssl rsa -pubout failed");
        let pub_pem = String::from_utf8(pub_out.stdout).unwrap();
        (priv_pem, pub_pem)
    }

    #[test]
    fn mint_jwt_decodes_with_matching_public_key() {
        let (priv_pem, pub_pem) = generate_test_rsa_keypair();
        let auth = GithubAppAuth::new(123456, priv_pem.as_bytes()).unwrap();
        let jwt = auth.mint_app_jwt().unwrap();

        let decoding_key = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).unwrap();
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_required_spec_claims(&["iat", "exp", "iss"]);

        #[derive(Deserialize)]
        struct Claims {
            iat: u64,
            exp: u64,
            iss: String,
        }
        let decoded = decode::<Claims>(&jwt, &decoding_key, &validation).unwrap();
        assert_eq!(decoded.claims.iss, "123456");
        assert!(decoded.claims.exp > decoded.claims.iat);
        // Lifetime: APP_JWT_LIFETIME + APP_JWT_IAT_BACKDATE = 10 min;
        // give 1 min margin for clock drift in the test runner.
        assert!(decoded.claims.exp - decoded.claims.iat <= 11 * 60);
    }

    #[test]
    fn rejects_garbage_private_key() {
        match GithubAppAuth::new(1, b"not a pem") {
            Err(GithubAppError::Key(_)) => {}
            Err(other) => panic!("expected Key error, got: {:?}", other),
            Ok(_) => panic!("expected Err, got Ok"),
        }
    }

    #[test]
    fn resolve_installation_id_uses_override() {
        let r = resolve_installation_id(Some(42), "en").unwrap();
        assert_eq!(r, 42);
    }

    #[test]
    fn resolve_installation_id_errors_without_override_or_env() {
        std::env::remove_var("PANKOSMIA_DEFAULT_INSTALLATION_ID");
        let err = resolve_installation_id(None, "en").unwrap_err();
        assert!(matches!(err, GithubAppError::NoInstallationId(_)));
    }
}
