//! `BlobStore` — abstraction over object-storage uploads/downloads.
//!
//! Backwards-compat note: in single-tenant FS deployments the impl is
//! a thin wrapper that reads/writes files on disk. In hosted Phase 2
//! deployments the impl issues presigned URLs to Supabase Storage /
//! S3-compatible storage and never touches the bytes.
//!
//! This module ships the trait only; implementations land in M9.
//! Defining the trait now keeps the audio-related endpoints' future
//! signatures clear from the start.
//!
//! See `docs/SCALING.md` §2 for why audio bytes do not transit the
//! Rust server in production.

use crate::identity::{LanguageCode, UserId};
use crate::store::types::*;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;

/// Async byte-stream type for streaming downloads. Implementations
/// must use streaming (not whole-buffer) for anything user-uploadable
/// — multi-GB audio uploads would OOM a buffered impl.
pub type ByteStream = BoxStream<'static, Result<Bytes, std::io::Error>>;

#[async_trait]
pub trait BlobStore: Send + Sync {
    // --- whole-buffer methods -------------------------------------
    //
    // Reserved for bounded payloads: metadata.json, settings.json,
    // small ingredient text. Anything user-supplied (audio, video,
    // zips) MUST go through the streaming variants.

    async fn put_blob(&self, lang: LanguageCode, key: BlobKey, bytes: Bytes) -> StoreResult<()>;
    async fn get_blob(&self, lang: LanguageCode, key: BlobKey) -> StoreResult<Bytes>;
    async fn delete_blob(&self, lang: LanguageCode, key: BlobKey) -> StoreResult<()>;

    // --- streaming methods ----------------------------------------

    async fn get_blob_stream(&self, lang: LanguageCode, key: BlobKey) -> StoreResult<ByteStream>;

    // --- temp-upload flow -----------------------------------------
    //
    // Phase 1: bytes flow through the server, store in a tempfile.
    // Phase 2: server issues a presigned PUT URL; the browser uploads
    // direct, then calls back to finalize.

    async fn put_temp(&self, user: UserId) -> StoreResult<(TempId, TempUploadHandle)>;

    async fn take_temp(&self, user: UserId, id: TempId) -> StoreResult<Bytes>;
}
