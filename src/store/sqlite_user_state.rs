//! SQLite-backed per-user state persistence.
//!
//! Two tables:
//!   - `user_state` — global per-user data (selected languages, i18n).
//!   - `user_language_state` — per-(user, language) data (BCV cursor,
//!     typography, current project).
//!
//! Both use a key-value layout: the `value` column is JSON text.
//! This avoids schema migrations when new state keys are added.
//!
//! Activated in GitHub mode when `PANKOSMIA_SQLITE_PATH` is set
//! (e.g. `/data/pankosmia-user-state.db`). When unset, the GitHub
//! store falls back to its existing in-memory defaults.

use crate::identity::{LanguageCode, UserId};
use crate::store::types::*;
use parking_lot::Mutex;
use rusqlite::Connection;
use std::path::Path;

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS user_state (
    user_id  TEXT NOT NULL,
    key      TEXT NOT NULL,
    value    TEXT NOT NULL,
    PRIMARY KEY (user_id, key)
);

CREATE TABLE IF NOT EXISTS user_language_state (
    user_id  TEXT NOT NULL,
    lang     TEXT NOT NULL,
    key      TEXT NOT NULL,
    value    TEXT NOT NULL,
    PRIMARY KEY (user_id, lang, key)
);
";

pub struct SqliteUserState {
    conn: Mutex<Connection>,
}

impl SqliteUserState {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)
            .map_err(|e| StoreError::Backend(format!("sqlite open: {}", e)))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| StoreError::Backend(format!("sqlite pragma: {}", e)))?;
        conn.execute_batch(SCHEMA_SQL)
            .map_err(|e| StoreError::Backend(format!("sqlite schema: {}", e)))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // -- global per-user state ----------------------------------------

    fn get_user(&self, user: &UserId, key: &str) -> StoreResult<Option<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare_cached("SELECT value FROM user_state WHERE user_id = ?1 AND key = ?2")
            .map_err(map_err)?;
        let result = stmt
            .query_row(rusqlite::params![user.0.to_string(), key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(map_err)?;
        Ok(result)
    }

    fn put_user(&self, user: &UserId, key: &str, value: &str) -> StoreResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO user_state (user_id, key, value) VALUES (?1, ?2, ?3)",
            rusqlite::params![user.0.to_string(), key, value],
        )
        .map_err(map_err)?;
        Ok(())
    }

    // -- per-(user, language) state -----------------------------------

    fn get_user_lang(&self, user: &UserId, lang: &str, key: &str) -> StoreResult<Option<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare_cached(
                "SELECT value FROM user_language_state WHERE user_id = ?1 AND lang = ?2 AND key = ?3",
            )
            .map_err(map_err)?;
        let result = stmt
            .query_row(rusqlite::params![user.0.to_string(), lang, key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(map_err)?;
        Ok(result)
    }

    fn put_user_lang(&self, user: &UserId, lang: &str, key: &str, value: &str) -> StoreResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO user_language_state (user_id, lang, key, value) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![user.0.to_string(), lang, key, value],
        )
        .map_err(map_err)?;
        Ok(())
    }

    // -- typed accessors used by GitHubLanguageStore -------------------

    pub fn get_languages(&self, user: &UserId) -> StoreResult<Vec<LanguageCode>> {
        match self.get_user(user, "languages")? {
            Some(json) => {
                let codes: Vec<String> =
                    serde_json::from_str(&json).map_err(|e| StoreError::Json(e))?;
                codes
                    .into_iter()
                    .map(|s| {
                        LanguageCode::parse(&s)
                            .map_err(|e| StoreError::Invalid(format!("bad language code: {}", e)))
                    })
                    .collect()
            }
            None => Ok(Vec::new()),
        }
    }

    pub fn put_languages(&self, user: &UserId, langs: &[LanguageCode]) -> StoreResult<()> {
        let codes: Vec<&str> = langs.iter().map(|l| l.as_str()).collect();
        let json = serde_json::to_string(&codes)?;
        self.put_user(user, "languages", &json)
    }

    pub fn get_typography(
        &self,
        user: &UserId,
        lang: &LanguageCode,
    ) -> StoreResult<Option<Typography>> {
        match self.get_user_lang(user, lang.as_str(), "typography")? {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    pub fn put_typography(
        &self,
        user: &UserId,
        lang: &LanguageCode,
        t: &Typography,
    ) -> StoreResult<()> {
        let json = serde_json::to_string(t)?;
        self.put_user_lang(user, lang.as_str(), "typography", &json)
    }

    pub fn get_bcv(&self, user: &UserId, lang: &LanguageCode) -> StoreResult<Option<Bcv>> {
        match self.get_user_lang(user, lang.as_str(), "bcv")? {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    pub fn put_bcv(&self, user: &UserId, lang: &LanguageCode, bcv: &Bcv) -> StoreResult<()> {
        let json = serde_json::to_string(bcv)?;
        self.put_user_lang(user, lang.as_str(), "bcv", &json)
    }

    pub fn get_app_state(&self, lang: &LanguageCode) -> StoreResult<Option<AppState>> {
        let key = format!("app_state:{}", lang.as_str());
        match self.get_user(&UserId::nil(), &key)? {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    pub fn put_app_state(&self, lang: &LanguageCode, s: &AppState) -> StoreResult<()> {
        let key = format!("app_state:{}", lang.as_str());
        let json = serde_json::to_string(s)?;
        self.put_user(&UserId::nil(), &key, &json)
    }

    // -- user language claims -------------------------------------------

    pub fn get_claimed_languages(&self, user: &UserId) -> StoreResult<Vec<LanguageCode>> {
        match self.get_user(user, "claimed_languages")? {
            Some(json) => {
                let codes: Vec<String> =
                    serde_json::from_str(&json).map_err(|e| StoreError::Json(e))?;
                codes
                    .into_iter()
                    .map(|s| {
                        LanguageCode::parse(&s)
                            .map_err(|e| StoreError::Invalid(format!("bad language code: {}", e)))
                    })
                    .collect()
            }
            None => Ok(Vec::new()),
        }
    }

    pub fn put_claimed_languages(&self, user: &UserId, langs: &[LanguageCode]) -> StoreResult<()> {
        let codes: Vec<&str> = langs.iter().map(|l| l.as_str()).collect();
        let json = serde_json::to_string(&codes)?;
        self.put_user(user, "claimed_languages", &json)
    }

    pub fn get_current_language(&self, user: &UserId) -> StoreResult<Option<LanguageCode>> {
        match self.get_user(user, "current_language")? {
            Some(s) => Ok(Some(LanguageCode::parse(&s).map_err(|e| {
                StoreError::Invalid(format!("bad language code: {}", e))
            })?)),
            None => Ok(None),
        }
    }

    pub fn put_current_language(&self, user: &UserId, lang: &LanguageCode) -> StoreResult<()> {
        self.put_user(user, "current_language", lang.as_str())
    }

    pub fn clear_current_language(&self, user: &UserId) -> StoreResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM user_state WHERE user_id = ?1 AND key = ?2",
            rusqlite::params![user.0.to_string(), "current_language"],
        )
        .map_err(map_err)?;
        Ok(())
    }

    // -- auth tokens -----------------------------------------------------

    pub fn get_auth_token(&self, user: &UserId, key: &str) -> StoreResult<Option<String>> {
        let db_key = format!("auth_token:{}", key);
        self.get_user(user, &db_key)
    }

    pub fn put_auth_token(&self, user: &UserId, key: &str, code: &str) -> StoreResult<()> {
        let db_key = format!("auth_token:{}", key);
        self.put_user(user, &db_key, code)
    }

    pub fn delete_auth_token(&self, user: &UserId, key: &str) -> StoreResult<()> {
        let db_key = format!("auth_token:{}", key);
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM user_state WHERE user_id = ?1 AND key = ?2",
            rusqlite::params![user.0.to_string(), &db_key],
        )
        .map_err(map_err)?;
        Ok(())
    }

    pub fn put_auth_request(&self, user: &UserId, key: &str, req: &AuthRequest) -> StoreResult<()> {
        let db_key = format!("auth_req:{}", key);
        let json = serde_json::to_string(req)?;
        self.put_user(user, &db_key, &json)
    }

    pub fn take_auth_request(&self, user: &UserId, key: &str) -> StoreResult<Option<AuthRequest>> {
        let db_key = format!("auth_req:{}", key);
        let val = self.get_user(user, &db_key)?;
        if val.is_some() {
            let conn = self.conn.lock();
            conn.execute(
                "DELETE FROM user_state WHERE user_id = ?1 AND key = ?2",
                rusqlite::params![user.0.to_string(), &db_key],
            )
            .map_err(map_err)?;
        }
        match val {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }
}

use rusqlite::OptionalExtension;

fn map_err(e: rusqlite::Error) -> StoreError {
    StoreError::Backend(format!("sqlite: {}", e))
}
