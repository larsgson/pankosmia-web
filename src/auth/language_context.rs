//! `LanguageHeader` request guard — extracts and validates the
//! `X-Language-Code` header.
//!
//! Used by save endpoints under the GitHub backend to learn which
//! language a request is targeting (the legacy `<repo_path..>` URL
//! segments aren't meaningful in the GitHub model).

use crate::identity::LanguageCode;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};

/// Lightweight guard: extract + parse `X-Language-Code`. Forwards
/// (rather than errors) on a missing header so endpoints can take
/// `Option<LanguageHeader>` and decide what to do in each backend.
#[derive(Clone, Debug)]
pub struct LanguageHeader(pub LanguageCode);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for LanguageHeader {
    type Error = LanguageHeaderError;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match req.headers().get_one("X-Language-Code") {
            Some(s) => match LanguageCode::parse(s) {
                Ok(l) => Outcome::Success(LanguageHeader(l)),
                Err(_) => Outcome::Error((
                    Status::BadRequest,
                    LanguageHeaderError::Invalid(s.to_string()),
                )),
            },
            None => Outcome::Forward(Status::BadRequest),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LanguageHeaderError {
    #[error("invalid X-Language-Code: {0}")]
    Invalid(String),
}
