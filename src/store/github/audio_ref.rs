//! Audio reference (`audio_content/**/ref.json`) validation.
//!
//! Audio bytes live outside this server (Internet Archive primary,
//! any CC-licensed URL secondary — see `docs/impl/AUDIO_STRATEGY.md`).
//! What lands in the burrito is a tiny JSON file describing where the
//! audio lives plus license / metadata. This module validates that
//! file before the save endpoint pushes it through to GitHub.
//!
//! Two schema shapes supported (both `schema_version: 1`):
//!
//! - **Flat / single-take**: one `url`/`type`/`license` triple at the
//!   top level.
//! - **Multi-take**: a `takes` array with a `main_take_index`,
//!   for OBS-style "record N takes per paragraph" workflows.
//!
//! Validation also enforces a license allowlist (operator-configurable
//! via `PANKOSMIA_ALLOWED_LICENSES`), an optional host allowlist
//! (`PANKOSMIA_AUDIO_URL_HOSTS_ALLOWLIST`), and an opt-in
//! HEAD-reachability check (`PANKOSMIA_VALIDATE_AUDIO_URLS=true`).

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Built-in default license allowlist. Operators can override via
/// `PANKOSMIA_ALLOWED_LICENSES` (comma-separated SPDX IDs; `*`
/// disables the check).
pub const DEFAULT_ALLOWED_LICENSES: &[&str] = &[
    "CC0-1.0",
    "CC-BY-4.0",
    "CC-BY-SA-4.0",
    "CC-BY-NC-4.0",
    "CC-BY-ND-4.0",
    "CC-BY-NC-SA-4.0",
    "CC-BY-NC-ND-4.0",
    "Public-Domain",
];

/// Content-Type signal that explicitly marks a write as an audio
/// reference (alternative to detection by path).
pub const AUDIO_REF_CONTENT_TYPE: &str = "application/vnd.pankosmia.audio-ref+json";

/// HEAD-validation timeout when `PANKOSMIA_VALIDATE_AUDIO_URLS=true`.
const HEAD_VALIDATION_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum AudioRefError {
    #[error("invalid JSON: {0}")]
    BadJson(String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("invalid {field}: {reason}")]
    BadField {
        field: &'static str,
        reason: String,
    },
    #[error("license '{0}' not in allowed-licenses list")]
    DisallowedLicense(String),
    #[error("URL host '{0}' not in PANKOSMIA_AUDIO_URL_HOSTS_ALLOWLIST")]
    DisallowedHost(String),
    #[error("URL does not return audio content")]
    UrlNotAudio,
    #[error("schema_version {0} not supported")]
    UnsupportedSchemaVersion(u32),
    #[error("either `url` / `type` / `license` (flat shape) OR `takes` (multi-take shape) required")]
    NeitherShape,
    #[error("`main_take_index` out of range for `takes` length")]
    MainTakeOutOfRange,
}

/// Configuration loaded from env once at startup; managed as Rocket
/// state for cheap per-request access.
#[derive(Clone, Debug)]
pub struct AudioRefConfig {
    /// Empty Vec + `validate_against_allowlist = false` means "any
    /// license accepted".
    pub allowed_licenses: Vec<String>,
    pub validate_against_allowlist: bool,
    /// Optional host allowlist (empty = any host).
    pub allowed_hosts: Vec<String>,
    /// Whether to HEAD-validate audio URLs on write.
    pub validate_urls: bool,
}

impl AudioRefConfig {
    pub fn from_env() -> Self {
        let raw = std::env::var("PANKOSMIA_ALLOWED_LICENSES").unwrap_or_default();
        let (allowed_licenses, validate_against_allowlist) = if raw == "*" {
            (vec![], false)
        } else if raw.is_empty() {
            (
                DEFAULT_ALLOWED_LICENSES
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                true,
            )
        } else {
            (
                raw.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                true,
            )
        };
        let allowed_hosts = std::env::var("PANKOSMIA_AUDIO_URL_HOSTS_ALLOWLIST")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|h| h.trim().to_string())
                    .filter(|h| !h.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let validate_urls = std::env::var("PANKOSMIA_VALIDATE_AUDIO_URLS")
            .map(|v| matches!(v.as_str(), "true" | "1" | "yes"))
            .unwrap_or(false);
        AudioRefConfig {
            allowed_licenses,
            validate_against_allowlist,
            allowed_hosts,
            validate_urls,
        }
    }
}

/// Detect "this write looks like an audio reference" by path.
/// The Content-Type signal is checked separately at the endpoint.
pub fn is_audio_ref_path(ipath: &str) -> bool {
    // Match: audio_content/<anything>/ref.json (anywhere under
    // audio_content) OR *.audioref extension as a future-friendly
    // alternative.
    if ipath.ends_with(".audioref") {
        return true;
    }
    if !ipath.starts_with("audio_content/") {
        return false;
    }
    ipath.ends_with("/ref.json") || ipath == "audio_content/ref.json"
}

#[derive(Deserialize, Serialize, Debug)]
struct RefV1Raw {
    schema_version: Option<u32>,
    // Flat shape:
    url: Option<String>,
    #[serde(rename = "type")]
    mime_type: Option<String>,
    license: Option<String>,
    // Multi-take shape:
    takes: Option<Vec<RefV1Take>>,
    main_take_index: Option<usize>,
}

#[derive(Deserialize, Serialize, Debug)]
struct RefV1Take {
    url: String,
    #[serde(rename = "type")]
    mime_type: String,
    license: String,
    // ignore the rest at validation time
}

/// Validate raw JSON bytes against the v1 audio-reference schema and
/// the operator-configured policy. Does NOT perform the optional
/// network HEAD check — call `head_validate_url` separately when
/// `cfg.validate_urls` is true.
pub fn validate_schema(bytes: &[u8], cfg: &AudioRefConfig) -> Result<(), AudioRefError> {
    let parsed: RefV1Raw = serde_json::from_slice(bytes)
        .map_err(|e| AudioRefError::BadJson(e.to_string()))?;

    let version = parsed.schema_version.unwrap_or(1);
    if version != 1 {
        return Err(AudioRefError::UnsupportedSchemaVersion(version));
    }

    // Disambiguate flat vs multi-take. Allow both keys present
    // (degenerate case = one entry in `takes` matching the flat
    // triple), but at minimum require one of the two shapes to
    // present a valid entry to check.
    let mut to_validate: Vec<(&str, &str, &str)> = Vec::new();
    if let Some(takes) = &parsed.takes {
        if takes.is_empty() {
            return Err(AudioRefError::BadField {
                field: "takes",
                reason: "must be a non-empty array".into(),
            });
        }
        let main_idx = parsed.main_take_index.unwrap_or(0);
        if main_idx >= takes.len() {
            return Err(AudioRefError::MainTakeOutOfRange);
        }
        for t in takes {
            to_validate.push((t.url.as_str(), t.mime_type.as_str(), t.license.as_str()));
        }
    } else {
        let url = parsed
            .url
            .as_deref()
            .ok_or(AudioRefError::MissingField("url"))?;
        let mt = parsed
            .mime_type
            .as_deref()
            .ok_or(AudioRefError::MissingField("type"))?;
        let lic = parsed
            .license
            .as_deref()
            .ok_or(AudioRefError::MissingField("license"))?;
        to_validate.push((url, mt, lic));
    }
    if to_validate.is_empty() {
        return Err(AudioRefError::NeitherShape);
    }

    for (url, mime_type, license) in &to_validate {
        validate_url_shape(url)?;
        validate_mime_type(mime_type)?;
        validate_license(license, cfg)?;
        if !cfg.allowed_hosts.is_empty() {
            validate_host(url, cfg)?;
        }
    }
    Ok(())
}

fn validate_url_shape(url: &str) -> Result<(), AudioRefError> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(AudioRefError::BadField {
            field: "url",
            reason: "must be http:// or https://".into(),
        });
    }
    Ok(())
}

fn validate_mime_type(t: &str) -> Result<(), AudioRefError> {
    if !t.starts_with("audio/") {
        return Err(AudioRefError::BadField {
            field: "type",
            reason: format!("'{}' is not an audio/* MIME type", t),
        });
    }
    Ok(())
}

fn validate_license(license: &str, cfg: &AudioRefConfig) -> Result<(), AudioRefError> {
    if !cfg.validate_against_allowlist {
        return Ok(());
    }
    if cfg
        .allowed_licenses
        .iter()
        .any(|l| l.eq_ignore_ascii_case(license))
    {
        return Ok(());
    }
    Err(AudioRefError::DisallowedLicense(license.to_string()))
}

fn validate_host(url: &str, cfg: &AudioRefConfig) -> Result<(), AudioRefError> {
    // Pull host out of the URL crudely; no full RFC 3986 parser
    // needed here.
    let after_scheme = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url);
    let host_and_after = after_scheme.split('/').next().unwrap_or("");
    let host = host_and_after.split(':').next().unwrap_or("");
    if cfg
        .allowed_hosts
        .iter()
        .any(|h| h.eq_ignore_ascii_case(host))
    {
        return Ok(());
    }
    Err(AudioRefError::DisallowedHost(host.to_string()))
}

/// HEAD-check a URL when `cfg.validate_urls` is true. Returns
/// `Ok(true)` if the URL responds 2xx with an audio Content-Type,
/// `Ok(false)` if it responds 2xx with a non-audio Content-Type,
/// and `Err(...)` only on network/timeout (caller decides whether
/// to treat that as "accept with warning" or "reject").
pub async fn head_validate_url(url: &str) -> Result<bool, AudioRefError> {
    let client = reqwest::Client::builder()
        .timeout(HEAD_VALIDATION_TIMEOUT)
        .build()
        .map_err(|e| AudioRefError::BadField {
            field: "url",
            reason: format!("HTTP client: {}", e),
        })?;
    let resp = client
        .head(url)
        .send()
        .await
        .map_err(|e| AudioRefError::BadField {
            field: "url",
            reason: format!("HEAD request failed: {}", e),
        })?;
    if !resp.status().is_success() {
        return Err(AudioRefError::BadField {
            field: "url",
            reason: format!("HEAD returned {}", resp.status().as_u16()),
        });
    }
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    Ok(ct.starts_with("audio/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow_all_cfg() -> AudioRefConfig {
        AudioRefConfig {
            allowed_licenses: vec![],
            validate_against_allowlist: false,
            allowed_hosts: vec![],
            validate_urls: false,
        }
    }

    fn default_cfg() -> AudioRefConfig {
        AudioRefConfig {
            allowed_licenses: DEFAULT_ALLOWED_LICENSES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            validate_against_allowlist: true,
            allowed_hosts: vec![],
            validate_urls: false,
        }
    }

    #[test]
    fn detects_audio_ref_path() {
        assert!(is_audio_ref_path("audio_content/01-01/ref.json"));
        assert!(is_audio_ref_path(
            "audio_content/some/deeply/nested/path/ref.json"
        ));
        assert!(is_audio_ref_path("audio_content/foo.audioref"));
        assert!(!is_audio_ref_path("ingredients/MAT/1.usfm"));
        assert!(!is_audio_ref_path("audio_content/01-01/audio.mp3"));
        assert!(!is_audio_ref_path("other_content/ref.json"));
    }

    #[test]
    fn accepts_minimal_flat_ref() {
        let body = br#"{
            "schema_version": 1,
            "url": "https://archive.org/download/x/y.mp3",
            "type": "audio/mp3",
            "license": "CC-BY-SA-4.0"
        }"#;
        validate_schema(body, &default_cfg()).unwrap();
    }

    #[test]
    fn rejects_missing_required_field() {
        let body = br#"{"schema_version":1,"url":"https://x/y.mp3","type":"audio/mp3"}"#;
        let err = validate_schema(body, &default_cfg()).unwrap_err();
        match err {
            AudioRefError::MissingField(f) => assert_eq!(f, "license"),
            other => panic!("expected MissingField('license'), got {:?}", other),
        }
    }

    #[test]
    fn rejects_bad_url_scheme() {
        let body = br#"{"schema_version":1,"url":"ftp://x/y.mp3","type":"audio/mp3","license":"CC0-1.0"}"#;
        let err = validate_schema(body, &default_cfg()).unwrap_err();
        assert!(matches!(err, AudioRefError::BadField { field: "url", .. }));
    }

    #[test]
    fn rejects_non_audio_mime() {
        let body = br#"{"schema_version":1,"url":"https://x/y.png","type":"image/png","license":"CC0-1.0"}"#;
        let err = validate_schema(body, &default_cfg()).unwrap_err();
        assert!(matches!(err, AudioRefError::BadField { field: "type", .. }));
    }

    #[test]
    fn rejects_disallowed_license() {
        let body = br#"{"schema_version":1,"url":"https://x/y.mp3","type":"audio/mp3","license":"Proprietary"}"#;
        let err = validate_schema(body, &default_cfg()).unwrap_err();
        match err {
            AudioRefError::DisallowedLicense(l) => assert_eq!(l, "Proprietary"),
            other => panic!("expected DisallowedLicense, got {:?}", other),
        }
    }

    #[test]
    fn allows_disallowed_license_when_allowlist_off() {
        let body = br#"{"schema_version":1,"url":"https://x/y.mp3","type":"audio/mp3","license":"Anything"}"#;
        validate_schema(body, &allow_all_cfg()).unwrap();
    }

    #[test]
    fn accepts_multi_take() {
        let body = br#"{
            "schema_version": 1,
            "takes": [
                {"url":"https://x/y_take1.mp3","type":"audio/mp3","license":"CC-BY-4.0","label":"take 1"},
                {"url":"https://x/y_take2.mp3","type":"audio/mp3","license":"CC-BY-4.0","label":"take 2"}
            ],
            "main_take_index": 1
        }"#;
        validate_schema(body, &default_cfg()).unwrap();
    }

    #[test]
    fn rejects_multi_take_out_of_range_main() {
        let body = br#"{
            "schema_version": 1,
            "takes": [
                {"url":"https://x/y.mp3","type":"audio/mp3","license":"CC-BY-4.0"}
            ],
            "main_take_index": 5
        }"#;
        let err = validate_schema(body, &default_cfg()).unwrap_err();
        assert!(matches!(err, AudioRefError::MainTakeOutOfRange));
    }

    #[test]
    fn rejects_multi_take_with_disallowed_license_in_one_entry() {
        let body = br#"{
            "schema_version": 1,
            "takes": [
                {"url":"https://x/y_1.mp3","type":"audio/mp3","license":"CC0-1.0"},
                {"url":"https://x/y_2.mp3","type":"audio/mp3","license":"Proprietary"}
            ],
            "main_take_index": 0
        }"#;
        let err = validate_schema(body, &default_cfg()).unwrap_err();
        assert!(matches!(err, AudioRefError::DisallowedLicense(_)));
    }

    #[test]
    fn enforces_host_allowlist_when_set() {
        let cfg = AudioRefConfig {
            allowed_licenses: vec!["CC0-1.0".to_string()],
            validate_against_allowlist: true,
            allowed_hosts: vec!["archive.org".to_string()],
            validate_urls: false,
        };
        let ok = br#"{"schema_version":1,"url":"https://archive.org/x/y.mp3","type":"audio/mp3","license":"CC0-1.0"}"#;
        validate_schema(ok, &cfg).unwrap();
        let bad = br#"{"schema_version":1,"url":"https://example.com/y.mp3","type":"audio/mp3","license":"CC0-1.0"}"#;
        let err = validate_schema(bad, &cfg).unwrap_err();
        assert!(matches!(err, AudioRefError::DisallowedHost(_)));
    }

    #[test]
    fn rejects_unsupported_schema_version() {
        let body = br#"{"schema_version":99,"url":"https://x/y.mp3","type":"audio/mp3","license":"CC0-1.0"}"#;
        let err = validate_schema(body, &default_cfg()).unwrap_err();
        assert!(matches!(err, AudioRefError::UnsupportedSchemaVersion(99)));
    }

    #[test]
    fn rejects_garbage_json() {
        let err = validate_schema(b"{not json", &default_cfg()).unwrap_err();
        assert!(matches!(err, AudioRefError::BadJson(_)));
    }
}
