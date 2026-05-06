//! S3-compatible image upload for Project Creatives.
//!
//! Phase 3.29. `POST /v1/projects/{projectId}/creatives/{id}/image`
//! accepts a multipart upload, sniffs the magic bytes, and writes
//! to the configured object store. The store backend is selected
//! by operator config (S3, MinIO, R2, GCS-via-S3-compat).
//!
//! Spec refs: `REQUIREMENTS.md` § 7.9, `API.md` § 3.5.

#![allow(dead_code)]

/// Maximum allowed payload size in bytes (40 MB per spec).
pub const MAX_BYTES: usize = 40 * 1024 * 1024;

/// Allowed MIME types per `REQUIREMENTS.md` § 7.9. SVG is
/// **not** in the list (script-execution risk); HEIC/HEIF and
/// BMP/TIFF are also rejected.
pub const ALLOWED_MIME: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/avif",
];

/// Magic-byte sniffing. Returns the canonical MIME type if the
/// first few bytes of `buf` match a recognized image format,
/// `None` otherwise.
pub fn sniff_mime(buf: &[u8]) -> Option<&'static str> {
    if buf.len() < 12 {
        return None;
    }
    // JPEG: FF D8 FF
    if buf[0] == 0xFF && buf[1] == 0xD8 && buf[2] == 0xFF {
        return Some("image/jpeg");
    }
    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if &buf[0..8] == b"\x89PNG\r\n\x1a\n" {
        return Some("image/png");
    }
    // GIF: "GIF87a" or "GIF89a"
    if &buf[0..6] == b"GIF87a" || &buf[0..6] == b"GIF89a" {
        return Some("image/gif");
    }
    // WebP: "RIFF" .... "WEBP"
    if &buf[0..4] == b"RIFF" && &buf[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    // AVIF: "....ftypavif" — the 4-byte size prefix at the
    // head means we look at offset 4 for "ftyp" and offset 8
    // for "avif".
    if &buf[4..8] == b"ftyp" && &buf[8..12] == b"avif" {
        return Some("image/avif");
    }
    None
}

/// Errors raised by the upload handler. Each maps onto a
/// canonical HTTP status / error code surface.
#[derive(Debug, PartialEq, Eq)]
pub enum UploadError {
    /// Body exceeds `MAX_BYTES`.
    PayloadTooLarge,
    /// `Content-Type`/extension claims one image format, magic
    /// bytes claim another (or magic bytes don't match any
    /// allowed format).
    UnsupportedMediaType,
    /// `Content-Type` claims an image format that's outside
    /// the allowed list (e.g. `image/svg+xml`).
    DisallowedMime,
}

impl UploadError {
    pub fn http_status(&self) -> u16 {
        match self {
            UploadError::PayloadTooLarge => 413,
            UploadError::UnsupportedMediaType => 415,
            UploadError::DisallowedMime => 415,
        }
    }
    pub fn code(&self) -> &'static str {
        match self {
            UploadError::PayloadTooLarge => "payload_too_large",
            UploadError::UnsupportedMediaType => "unsupported_media_type",
            UploadError::DisallowedMime => "disallowed_mime",
        }
    }
}

/// Storage backend trait. v0 ships an in-memory adapter for
/// tests; S3 / MinIO / GCS adapters live behind the same trait.
/// `put` returns the URL the caller should serialize back as
/// `imageUrl`.
#[async_trait::async_trait]
pub trait ImageStore: Send + Sync {
    async fn put(&self, key: &str, mime: &str, bytes: &[u8]) -> anyhow::Result<String>;
}

/// In-memory adapter. Useful for unit and integration tests
/// without an S3 dependency.
pub struct InMemoryStore {
    objects: tokio::sync::Mutex<std::collections::HashMap<String, (String, Vec<u8>)>>,
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self {
            objects: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl ImageStore for InMemoryStore {
    async fn put(&self, key: &str, mime: &str, bytes: &[u8]) -> anyhow::Result<String> {
        let mut g = self.objects.lock().await;
        g.insert(key.into(), (mime.into(), bytes.to_vec()));
        Ok(format!("memory://{key}"))
    }
}

/// Compose the storage key per `REQUIREMENTS.md` § 7.9
/// "Naming": `projects/{project_id}/creatives/{creative_id}/{uuid}.{ext}`.
pub fn storage_key(project_id: &str, creative_id: i64, uuid: &str, mime: &str) -> String {
    let ext = match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        _ => "bin",
    };
    format!("projects/{project_id}/creatives/{creative_id}/{uuid}.{ext}")
}

/// Validate the upload body against the spec's constraints.
/// Returns the canonical MIME type (from magic bytes) on
/// success.
pub fn validate(declared_mime: Option<&str>, body: &[u8]) -> Result<&'static str, UploadError> {
    if body.len() > MAX_BYTES {
        return Err(UploadError::PayloadTooLarge);
    }
    let sniffed = sniff_mime(body).ok_or(UploadError::UnsupportedMediaType)?;
    if !ALLOWED_MIME.contains(&sniffed) {
        return Err(UploadError::DisallowedMime);
    }
    if let Some(declared) = declared_mime {
        // Mismatch between declared and sniffed = `415` per spec.
        if declared != sniffed {
            return Err(UploadError::UnsupportedMediaType);
        }
    }
    Ok(sniffed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg_bytes() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        v.resize(64, 0);
        v
    }
    fn png_bytes() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.resize(64, 0);
        v
    }
    fn gif_bytes() -> Vec<u8> {
        let mut v = b"GIF89a".to_vec();
        v.resize(64, 0);
        v
    }
    fn webp_bytes() -> Vec<u8> {
        let mut v = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        v.resize(64, 0);
        v
    }
    fn avif_bytes() -> Vec<u8> {
        let mut v = vec![0; 4]; // size prefix
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(b"avif");
        v.resize(64, 0);
        v
    }
    fn svg_bytes() -> Vec<u8> {
        b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec()
    }

    #[test]
    fn sniffs_each_allowed_format() {
        assert_eq!(sniff_mime(&jpeg_bytes()), Some("image/jpeg"));
        assert_eq!(sniff_mime(&png_bytes()), Some("image/png"));
        assert_eq!(sniff_mime(&gif_bytes()), Some("image/gif"));
        assert_eq!(sniff_mime(&webp_bytes()), Some("image/webp"));
        assert_eq!(sniff_mime(&avif_bytes()), Some("image/avif"));
    }

    #[test]
    fn svg_is_rejected() {
        let body = svg_bytes();
        let r = validate(Some("image/svg+xml"), &body);
        assert!(matches!(r, Err(UploadError::UnsupportedMediaType)));
    }

    #[test]
    fn declared_mismatch_is_415() {
        let body = png_bytes();
        let r = validate(Some("image/jpeg"), &body);
        assert_eq!(r, Err(UploadError::UnsupportedMediaType));
    }

    #[test]
    fn validate_happy_path_returns_canonical() {
        let body = jpeg_bytes();
        let m = validate(Some("image/jpeg"), &body).unwrap();
        assert_eq!(m, "image/jpeg");
    }

    #[test]
    fn payload_too_large_rejected() {
        let body = vec![0xFFu8; MAX_BYTES + 1];
        let r = validate(Some("image/jpeg"), &body);
        assert_eq!(r, Err(UploadError::PayloadTooLarge));
    }

    #[test]
    fn storage_key_format() {
        let k = storage_key("pj_a", 42, "u1", "image/png");
        assert_eq!(k, "projects/pj_a/creatives/42/u1.png");
    }

    #[tokio::test]
    async fn in_memory_store_put_round_trip() {
        let s = InMemoryStore::default();
        let url = s.put("k1", "image/png", b"data").await.unwrap();
        assert_eq!(url, "memory://k1");
    }

    #[test]
    fn http_status_mapping() {
        assert_eq!(UploadError::PayloadTooLarge.http_status(), 413);
        assert_eq!(UploadError::UnsupportedMediaType.http_status(), 415);
        assert_eq!(UploadError::DisallowedMime.http_status(), 415);
    }
}
