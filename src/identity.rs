//! Strongly-typed identifiers used across the Phase 2 multi-tenant
//! surface.
//!
//! These newtypes intentionally do not carry any storage logic — they
//! are *just* identifiers. Resolution into filesystem paths or
//! database rows lives in `crate::store`.
//!
//! Backwards compat: in single-tenant FS deployments these resolve to
//! defaults (the "local" user and the configured default language).
//! See `docs/PHASE2_DESIGN.md` §11 for the resolved semantics.

use rocket::request::FromParam;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Identifier for a user. In hosted Phase 2 deployments this is the
/// `sub` claim of a Supabase JWT (UUID). In single-tenant FS
/// deployments this is `UserId(Uuid::nil())`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(pub Uuid);

impl UserId {
    pub const fn nil() -> Self {
        UserId(Uuid::nil())
    }
    pub fn new() -> Self {
        UserId(Uuid::new_v4())
    }
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
    /// Deterministically derive a `UserId` from a GitHub user-id
    /// (i64). Same GitHub user always produces the same UUID, so
    /// existing per-user files keep working across restarts.
    /// UUIDv5 with a fixed `pankosmia.github` namespace.
    pub fn from_github_id(github_user_id: i64) -> Self {
        // Hardcoded namespace. Generated once with `uuid::Uuid::new_v4()`
        // and pasted; stable for the life of the project.
        const PANKOSMIA_GITHUB_NS: Uuid =
            Uuid::from_u128(0xb29c4dfa_4c5b_4f99_a23a_0d6c7c6c2bde_u128);
        let name = format!("github:{}", github_user_id);
        UserId(Uuid::new_v5(&PANKOSMIA_GITHUB_NS, name.as_bytes()))
    }
}

/// The "local user" stand-in used in single-tenant FS deployments
/// before `AuthUser` (M5) lands. Endpoints reach for this when they
/// would otherwise have a real `UserId` from a verified JWT. Hosted
/// Phase 2 deployments never see it — `AuthUser` resolves first.
pub const LOCAL_USER: UserId = UserId(Uuid::nil());

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Identifier for a single Git repository within a language. UUIDs
/// are used internally so that repo display names (`source/org/name`)
/// can be renamed without changing identity.
///
/// In v0.15.x this is unused; introduced here so `ProjectStore` can
/// be defined now without churn later.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepoId(pub Uuid);

impl RepoId {
    pub fn new() -> Self {
        RepoId(Uuid::new_v4())
    }
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Display for RepoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<'r> FromParam<'r> for RepoId {
    type Error = ParseError;
    fn from_param(param: &'r str) -> Result<Self, Self::Error> {
        Uuid::parse_str(param)
            .map(RepoId)
            .map_err(|_| ParseError("not a uuid"))
    }
}

/// Tenancy unit. A BCP 47 language tag, e.g. `en`, `fr-CA`, `zh-Hans`.
///
/// Validated at construction:
///   - non-empty.
///   - max 16 ASCII chars.
///   - alpha / digit / hyphen only.
///   - cannot start or end with a hyphen.
///   - no consecutive hyphens.
///
/// This is a deliberately conservative subset of BCP 47; we don't
/// need to accept extension subtags (`en-US-x-private`) or grandfathered
/// codes for the use case. Tighten further if needed.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LanguageCode(String);

impl LanguageCode {
    /// Construct a `LanguageCode` from a string, validating the BCP 47
    /// subset above. Returns `Err` on any rule violation.
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        if s.is_empty() {
            return Err(ParseError("empty language code"));
        }
        if s.len() > 16 {
            return Err(ParseError("language code too long"));
        }
        if s.starts_with('-') || s.ends_with('-') {
            return Err(ParseError("language code starts/ends with hyphen"));
        }
        if s.contains("--") {
            return Err(ParseError("language code has consecutive hyphens"));
        }
        for c in s.chars() {
            let ok = c.is_ascii_alphanumeric() || c == '-';
            if !ok {
                return Err(ParseError("language code has forbidden char"));
            }
        }
        Ok(LanguageCode(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LanguageCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'r> FromParam<'r> for LanguageCode {
    type Error = ParseError;
    fn from_param(param: &'r str) -> Result<Self, Self::Error> {
        LanguageCode::parse(param)
    }
}

/// Compact error type shared by all `FromParam` impls in this module.
#[derive(Debug)]
pub struct ParseError(pub &'static str);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_code_accepts_basic_tags() {
        assert!(LanguageCode::parse("en").is_ok());
        assert!(LanguageCode::parse("fr").is_ok());
        assert!(LanguageCode::parse("fr-CA").is_ok());
        assert!(LanguageCode::parse("zh-Hans").is_ok());
        assert!(LanguageCode::parse("ar").is_ok());
    }

    #[test]
    fn language_code_rejects_bad_input() {
        assert!(LanguageCode::parse("").is_err());
        assert!(LanguageCode::parse("-en").is_err());
        assert!(LanguageCode::parse("en-").is_err());
        assert!(LanguageCode::parse("en--US").is_err());
        assert!(LanguageCode::parse("en/US").is_err());
        assert!(LanguageCode::parse("../etc/passwd").is_err());
        assert!(LanguageCode::parse("en\0").is_err());
        assert!(LanguageCode::parse("en US").is_err());
        assert!(LanguageCode::parse(&"a".repeat(17)).is_err());
    }
}
