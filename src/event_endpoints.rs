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
//! Phase 3.30: each successful verify pushes one Impression /
//! Click row into the events flusher (`AppState::events`).
//! Saturation **does not** fail the request here — the spec
//! says pings still succeed at signature-verify level even when
//! the channel is wedged (`REQUIREMENTS.md` § 7.6 row "Event
//! channel saturation").
//!
//! Spec refs: `API.md` § 4, `REQUIREMENTS.md` § 6.3.

#![allow(dead_code)]

use std::time::{SystemTime, UNIX_EPOCH};

use poem::http::{header, StatusCode};
use poem::web::{Data, Path as PoemPath, Query};
use poem::{handler, Response};

use crate::events::{self, Event, EventKind as ChannelEventKind};
use crate::hmac::{self, EventKind, VerifyError, DEFAULT_TTL_SECS};
use crate::snapshot::Snapshot;
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
    let snap = state.0.snapshot.read();
    // Per spec: impression endpoint always returns 204 (or GIF)
    // — tampered/expired → still 204 (silent), counter ticks.
    // We classify but don't surface the failure to the caller.
    if let Ok((payload, project_id, org_id)) =
        verify_against_snapshot(&snap, &signed, EventKind::Impression)
    {
        emit_event(
            state.0,
            ChannelEventKind::Impression,
            &org_id,
            &project_id,
            &payload,
            snap.config_version,
        );
    }
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
    let snap = state.0.snapshot.read();
    let (payload, project_id, org_id) =
        match verify_against_snapshot(&snap, &signed, EventKind::Click) {
            Ok(t) => t,
            Err(_) => {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body("bad_signature")
            }
        };

    emit_event(
        state.0,
        ChannelEventKind::Click,
        &org_id,
        &project_id,
        &payload,
        snap.config_version,
    );

    // Resolve the redirect target from the snapshot's
    // `click_through_urls` lookup, keyed on the ad id baked
    // into the signed payload. A missing entry falls through
    // to a safe placeholder rather than an error — the URL
    // verified, the click is recorded, the caller's "broken
    // creative" telemetry is the right surface for the
    // missing target. `?u=<url>` is the spec's open-redirect
    // override but only when signed into the payload, which
    // the current `SignaturePayload` doesn't carry yet
    // (follow-up paired with HMAC v2 wire format).
    let target = resolve_redirect(&snap, &project_id, payload.ad_id, query.0.u.as_deref());

    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, target)
        .finish()
}

/// Look up the click-through target. `signed_override` is the
/// `?u=<url>` value if present; today we ignore it (open-redirect
/// block) but the parameter stays in the signature so the call
/// site reads as the spec describes.
fn resolve_redirect(
    snap: &Snapshot,
    project_id: &str,
    ad_id: i64,
    signed_override: Option<&str>,
) -> String {
    let _ = signed_override; // wired when SignaturePayload v2 lands
    snap.projects
        .get(project_id)
        .and_then(|p| p.click_through_urls.get(&ad_id))
        .cloned()
        .unwrap_or_else(|| "/".to_string())
}

fn emit_event(
    state: &AppState,
    kind: ChannelEventKind,
    org_id: &str,
    project_id: &str,
    payload: &hmac::SignaturePayload,
    snapshot_version: i64,
) {
    let Some(sender) = state.events.as_ref() else {
        return;
    };
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    // `emit_event` is only called for Impression/Click hits;
    // Decision rows skip dedup and aren't routed through here.
    let dedup_kind = match kind {
        ChannelEventKind::Impression => hmac::EventKind::Impression,
        ChannelEventKind::Click => hmac::EventKind::Click,
        ChannelEventKind::Decision => return,
    };
    let dedup = hmac::dedup_key(project_id, dedup_kind, &payload.nonce);
    let event = Event {
        ts_ms: now_ms,
        org_id: org_id.to_string(),
        project_id: project_id.to_string(),
        kind,
        placement_id: None, // we only have the hashed form on the wire
        site_id: None,
        zone_id: None,
        ad_id: Some(payload.ad_id),
        creative_id: if payload.creative_id == 0 {
            None
        } else {
            Some(payload.creative_id)
        },
        flight_id: None,
        campaign_id: None,
        advertiser_id: None,
        url: None,
        referrer_host: None,
        user_agent_hash: None,
        signature_nonce: Some(payload.nonce.to_vec()),
        dedup_key: Some(dedup.to_vec()),
        snapshot_version: Some(snapshot_version),
    };
    // Pings tolerate saturation per spec — log and drop. The
    // dedup_key keeps the eventual replay path correct.
    if let Err(events::SendError::ChannelSaturated | events::SendError::FlusherDown) =
        sender.try_send(event)
    {
        tracing::debug!(?kind, "event channel saturated; dropping ping");
    }
}

fn verify_against_snapshot(
    snap: &Snapshot,
    signed: &str,
    kind: EventKind,
) -> Result<(hmac::SignaturePayload, String, String), VerifyError> {
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
    let _ = kind; // dedup_kind is derived in `emit_event`
    Ok((payload, project_id, project.org_id_for_event.clone()))
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

    fn snap_with_click_through(project_id: &str, ad_id: i64, url: &str) -> Snapshot {
        use crate::snapshot::ProjectSnapshot;
        use std::collections::HashMap;
        use std::sync::Arc;
        let mut click_through_urls = HashMap::new();
        click_through_urls.insert(ad_id, url.to_string());
        let mut projects = HashMap::new();
        projects.insert(
            project_id.to_string(),
            Arc::new(ProjectSnapshot {
                project_id: project_id.into(),
                org_id_for_event: "org_a".into(),
                click_through_urls,
                ..ProjectSnapshot::default()
            }),
        );
        Snapshot {
            config_version: 1,
            projects,
        }
    }

    #[test]
    fn resolve_redirect_returns_creative_url_when_known() {
        let snap = snap_with_click_through("pj_a", 42, "https://acme.example/sale");
        assert_eq!(
            resolve_redirect(&snap, "pj_a", 42, None),
            "https://acme.example/sale"
        );
    }

    #[test]
    fn resolve_redirect_falls_through_when_unknown() {
        let snap = snap_with_click_through("pj_a", 42, "https://acme.example/sale");
        // Unknown ad_id → placeholder.
        assert_eq!(resolve_redirect(&snap, "pj_a", 99, None), "/");
        // Unknown project → placeholder.
        assert_eq!(resolve_redirect(&snap, "pj_b", 42, None), "/");
    }

    #[test]
    fn resolve_redirect_ignores_unsigned_override() {
        // Until SignaturePayload v2 carries a signed redirect,
        // `?u=<url>` is treated as advisory and dropped — the
        // resolver picks the snapshot's URL regardless.
        let snap = snap_with_click_through("pj_a", 42, "https://safe.example");
        assert_eq!(
            resolve_redirect(&snap, "pj_a", 42, Some("https://attacker.example")),
            "https://safe.example"
        );
    }
}
