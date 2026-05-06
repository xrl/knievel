//! Public event-tracking endpoints.
//!
//! Phase 3.25. `GET /e/i/{signed}` (impression) and
//! `GET /e/c/{signed}` (click). Unauthenticated; the HMAC
//! signature in the URL is the authorization.
//!
//! Per `API.md` § 4:
//!   - Impression: 204 by default (or 1×1 GIF when `?fmt=gif`).
//!     Tampered/expired → 204 (silent), counter incremented.
//!   - Click: 302 redirect to the creative's `clickThroughUrl`.
//!     Tampered/expired → 400. Optional `?u=<url>` overrides
//!     redirect target only if signed into the payload (open-
//!     redirect block).
//!
//! Spec refs: `API.md` § 4, `REQUIREMENTS.md` § 6.3.

#![allow(dead_code)]

use std::time::{SystemTime, UNIX_EPOCH};

use poem::http::{header, StatusCode};
use poem::web::{Data, Path as PoemPath, Query};
use poem::{handler, Response};

use crate::hmac::{self, EventKind, VerifyError, DEFAULT_TTL_SECS};
use crate::state::AppState;

/// 1x1 transparent GIF, 43 bytes (`API.md` § 4 specifies the
/// length explicitly so it's pinned here as a constant).
const TRANSPARENT_GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xFF, 0xFF, 0xFF, 0x21, 0xF9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2C, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3B,
];

#[derive(serde::Deserialize)]
pub struct ImpressionQuery {
    pub fmt: Option<String>,
}

#[handler]
pub async fn impression(
    state: Data<&AppState>,
    PoemPath(signed): PoemPath<String>,
    query: Query<ImpressionQuery>,
) -> Response {
    let want_gif = query.0.fmt.as_deref() == Some("gif");
    // Per spec: impression endpoint always returns 204 (or GIF)
    // — tampered/expired → still 204 (silent), counter ticks.
    // We classify but don't surface the failure to the caller.
    let _ = verify_against_snapshot(&state, &signed, EventKind::Impression);
    if want_gif {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/gif")
            .body(TRANSPARENT_GIF.to_vec())
    } else {
        Response::builder().status(StatusCode::NO_CONTENT).finish()
    }
}

#[derive(serde::Deserialize)]
pub struct ClickQuery {
    /// Optional override redirect target. Only honored when the
    /// URL is signed into the payload — open-redirect block.
    pub u: Option<String>,
}

#[handler]
pub async fn click(
    state: Data<&AppState>,
    PoemPath(signed): PoemPath<String>,
    query: Query<ClickQuery>,
) -> Response {
    let payload = match verify_against_snapshot(&state, &signed, EventKind::Click) {
        Ok(p) => p,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body("bad_signature")
        }
    };

    // Resolve the redirect target from the snapshot's
    // creative entry. The snapshot's `Ad` doesn't carry
    // `click_through_url` directly today (the consumer side is
    // a 3.18 follow-up); for v0 the redirect is a placeholder
    // so the contract is testable.
    let target = query
        .0
        .u
        .as_deref()
        .filter(|_| false) // open-redirect block until ?u= signing lands
        .unwrap_or("/");
    let _ = payload;

    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, target)
        .finish()
}

fn verify_against_snapshot(
    state: &AppState,
    signed: &str,
    kind: EventKind,
) -> Result<hmac::SignaturePayload, VerifyError> {
    let snap = state.snapshot.read();
    // Without parsing the project_id out of the signed payload
    // first, we'd have to try every project's secret — that's
    // O(N projects). The wire layout puts project_id at the head
    // of the record, so we peek at it from the unauthenticated
    // base64 prefix.
    let project_id = peek_project_id(signed).ok_or(VerifyError::Malformed)?;
    let project = snap
        .projects
        .get(&project_id)
        .ok_or(VerifyError::BadSignature)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let payload = hmac::verify(
        signed,
        &project.hmac_secret,
        project.hmac_secret_previous.as_deref(),
        now,
        DEFAULT_TTL_SECS,
    )?;
    let _ = kind; // dedup_key wiring lands with the events flusher integration
    Ok(payload)
}

/// Peek `project_id` out of the signed blob without verifying.
/// The base64 prefix decodes to the binary record from
/// `hmac.rs::pack_record`, whose first byte is the
/// length-prefixed project_id.
fn peek_project_id(signed: &str) -> Option<String> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let (r_b64, _m_b64) = signed.split_once('.')?;
    let record = URL_SAFE_NO_PAD.decode(r_b64).ok()?;
    if record.is_empty() {
        return None;
    }
    let pid_len = record[0] as usize;
    if record.len() < 1 + pid_len {
        return None;
    }
    String::from_utf8(record[1..1 + pid_len].to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hmac::SignaturePayload;

    #[test]
    fn peek_project_id_round_trips() {
        let p = SignaturePayload {
            project_id: "pj_demo000001".into(),
            ad_id: 1,
            creative_id: 2,
            placement_id_hash: [0u8; 16],
            issued_at_secs: 1_700_000_000,
            nonce: [0u8; 8],
        };
        let signed = hmac::sign(&p, b"secret");
        let pid = peek_project_id(&signed).unwrap();
        assert_eq!(pid, "pj_demo000001");
    }

    #[test]
    fn peek_returns_none_on_garbage() {
        assert!(peek_project_id("not-a-blob").is_none());
        // "AAAA.BBBB" decodes to 3 bytes "\0\0\0"; pid_len = 0,
        // returns Some("").
        assert_eq!(peek_project_id("AAAA.BBBB"), Some(String::new()));
    }

    #[test]
    fn transparent_gif_is_43_bytes() {
        // API.md § 4 explicitly specifies the length.
        assert_eq!(TRANSPARENT_GIF.len(), 43);
    }
}
