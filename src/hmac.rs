//! HMAC sign + verify for impression/click URLs.
//!
//! Phase 3.16. Pure-Rust, no DB. The decision endpoint
//! (Phase 3.18) calls `sign` to mint impression/click URLs; the
//! `/e/...` event endpoints (Phase 3.25) call `verify` against
//! the current and (during the 8-hour overlap) previous per-
//! project secret. `dedup_key` stays stable across rotation —
//! it's keyed on `project_id` itself, not on the rotating
//! signing secret, which is what makes "dedup spans rotation
//! cleanly" (`API.md` § 4 "Replay, dedup, and counts").
//!
//! Spec refs: `API.md` § 4 "Signature payload",
//! `REQUIREMENTS.md` § 6.3.

#![allow(dead_code)]

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Mac, SimpleHmac};
use sha2::Sha256;

type HmacSha256 = SimpleHmac<Sha256>;

/// Default per-URL TTL (24 h, `REQUIREMENTS.md` § 6.3).
pub const DEFAULT_TTL_SECS: u64 = 24 * 60 * 60;
/// Rotation overlap window (`REQUIREMENTS.md` § 6.3).
pub const ROTATION_OVERLAP_SECS: u64 = 8 * 60 * 60;

/// Event kind discriminator. Matches the prefix used in
/// `/e/i/...` (impression) and `/e/c/...` (click) URLs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EventKind {
    Impression,
    Click,
}

impl EventKind {
    pub fn as_byte(self) -> u8 {
        match self {
            EventKind::Impression => 1,
            EventKind::Click => 2,
        }
    }

    pub fn url_letter(self) -> &'static str {
        match self {
            EventKind::Impression => "i",
            EventKind::Click => "c",
        }
    }
}

/// The HMAC payload. `API.md` § 4 "Signature payload":
///
///   project_id | ad_id | creative_id | placement_id_hash | issued_at | nonce
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignaturePayload {
    pub project_id: String,
    pub ad_id: i64,
    pub creative_id: i64,
    /// Stable hash of the placement id from the request, so two
    /// hits on the same placement collide for dedup. We store the
    /// full 16 bytes of an HMAC-SHA256 keyed on project_id.
    pub placement_id_hash: [u8; 16],
    /// Unix epoch seconds when the URL was minted.
    pub issued_at_secs: u64,
    pub nonce: [u8; 8],
}

/// Errors raised by `verify`.
#[derive(Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// The encoded blob is shorter than the wire format requires
    /// or has an invalid base64 alphabet.
    Malformed,
    /// HMAC mismatch under both the current and (when supplied)
    /// previous secret. Includes "no signing secrets" trivially.
    BadSignature,
    /// `now - issued_at_secs > ttl_secs`.
    Expired,
}

/// Compose the binary record from `API.md` § 4. Stable across
/// versions: any change here is a breaking change to every
/// in-flight URL.
fn pack_record(p: &SignaturePayload) -> Vec<u8> {
    // project_id is variable length; we length-prefix it as u8
    // since project ids are small (`pj_` + 12 hex = 15 bytes).
    let pid = p.project_id.as_bytes();
    assert!(pid.len() <= u8::MAX as usize, "project_id too long");
    let mut out = Vec::with_capacity(1 + pid.len() + 8 + 8 + 16 + 8 + 8);
    out.push(pid.len() as u8);
    out.extend_from_slice(pid);
    out.extend_from_slice(&p.ad_id.to_be_bytes());
    out.extend_from_slice(&p.creative_id.to_be_bytes());
    out.extend_from_slice(&p.placement_id_hash);
    out.extend_from_slice(&p.issued_at_secs.to_be_bytes());
    out.extend_from_slice(&p.nonce);
    out
}

fn unpack_record(buf: &[u8]) -> Option<SignaturePayload> {
    if buf.is_empty() {
        return None;
    }
    let pid_len = buf[0] as usize;
    let mut idx = 1;
    if buf.len() < idx + pid_len + 8 + 8 + 16 + 8 + 8 {
        return None;
    }
    let project_id = String::from_utf8(buf[idx..idx + pid_len].to_vec()).ok()?;
    idx += pid_len;
    let ad_id = i64::from_be_bytes(buf[idx..idx + 8].try_into().ok()?);
    idx += 8;
    let creative_id = i64::from_be_bytes(buf[idx..idx + 8].try_into().ok()?);
    idx += 8;
    let mut placement_id_hash = [0u8; 16];
    placement_id_hash.copy_from_slice(&buf[idx..idx + 16]);
    idx += 16;
    let issued_at_secs = u64::from_be_bytes(buf[idx..idx + 8].try_into().ok()?);
    idx += 8;
    let mut nonce = [0u8; 8];
    nonce.copy_from_slice(&buf[idx..idx + 8]);
    Some(SignaturePayload {
        project_id,
        ad_id,
        creative_id,
        placement_id_hash,
        issued_at_secs,
        nonce,
    })
}

/// Sign the payload with `secret`, returning the URL-safe
/// base64 (`<record>.<mac>`). The `.` separator lets `verify`
/// scan for the boundary cheaply.
pub fn sign(payload: &SignaturePayload, secret: &[u8]) -> String {
    let record = pack_record(payload);
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&record);
    let tag = mac.finalize().into_bytes();
    let r = URL_SAFE_NO_PAD.encode(&record);
    let m = URL_SAFE_NO_PAD.encode(tag);
    format!("{r}.{m}")
}

/// Verify `signed` against `current_secret`; if that fails and
/// `previous_secret` is supplied, fall back to it (rotation
/// overlap). On success returns the parsed payload after
/// checking TTL against `now_secs`.
pub fn verify(
    signed: &str,
    current_secret: &[u8],
    previous_secret: Option<&[u8]>,
    now_secs: u64,
    ttl_secs: u64,
) -> Result<SignaturePayload, VerifyError> {
    let (r_b64, m_b64) = signed.split_once('.').ok_or(VerifyError::Malformed)?;
    let record = URL_SAFE_NO_PAD
        .decode(r_b64)
        .map_err(|_| VerifyError::Malformed)?;
    let mac_bytes = URL_SAFE_NO_PAD
        .decode(m_b64)
        .map_err(|_| VerifyError::Malformed)?;
    let payload = unpack_record(&record).ok_or(VerifyError::Malformed)?;

    let mut ok = mac_matches(&record, &mac_bytes, current_secret);
    if !ok {
        if let Some(prev) = previous_secret {
            ok = mac_matches(&record, &mac_bytes, prev);
        }
    }
    if !ok {
        return Err(VerifyError::BadSignature);
    }

    if now_secs.saturating_sub(payload.issued_at_secs) > ttl_secs {
        return Err(VerifyError::Expired);
    }
    Ok(payload)
}

fn mac_matches(record: &[u8], expected: &[u8], secret: &[u8]) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else {
        return false;
    };
    mac.update(record);
    mac.verify_slice(expected).is_ok()
}

/// `dedup_key` per `API.md` § 4 "Replay, dedup, and counts" —
/// truncated HMAC-SHA256 to 16 bytes. The key is the
/// project_id itself rather than the rotating signing secret;
/// that's what makes the dedup_key stable across rotation
/// (the rotated signing secret would change the HMAC tag, so
/// reusing it would reset dedup at the rotation boundary,
/// which is exactly the bug the spec calls out).
pub fn dedup_key(project_id: &str, kind: EventKind, nonce: &[u8; 8]) -> [u8; 16] {
    let mut mac =
        HmacSha256::new_from_slice(project_id.as_bytes()).expect("HMAC accepts any key length");
    mac.update(&[kind.as_byte()]);
    mac.update(nonce);
    let tag = mac.finalize().into_bytes();
    let mut out = [0u8; 16];
    out.copy_from_slice(&tag[..16]);
    out
}

/// Helper: hash a placement id (caller-supplied string) into the
/// 16-byte field carried in the signature payload. Keyed on
/// project_id so two projects can't collide at the row level.
pub fn placement_id_hash(project_id: &str, placement_id: &str) -> [u8; 16] {
    let mut mac =
        HmacSha256::new_from_slice(project_id.as_bytes()).expect("HMAC accepts any key length");
    mac.update(placement_id.as_bytes());
    let tag = mac.finalize().into_bytes();
    let mut out = [0u8; 16];
    out.copy_from_slice(&tag[..16]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> SignaturePayload {
        SignaturePayload {
            project_id: "pj_demo000001".into(),
            ad_id: 12_345,
            creative_id: 67_890,
            placement_id_hash: [9u8; 16],
            issued_at_secs: 1_700_000_000,
            nonce: [0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04],
        }
    }

    #[test]
    fn sign_verify_round_trip() {
        let p = payload();
        let secret = b"secret-1";
        let s = sign(&p, secret);
        let now = p.issued_at_secs + 100;
        let parsed = verify(&s, secret, None, now, DEFAULT_TTL_SECS).unwrap();
        assert_eq!(parsed, p);
    }

    #[test]
    fn tampered_signature_rejected() {
        let p = payload();
        let secret = b"secret-1";
        let mut s = sign(&p, secret);
        // Flip a char near the end of the MAC.
        let last = s.pop().unwrap();
        s.push(if last == 'A' { 'B' } else { 'A' });
        let r = verify(&s, secret, None, p.issued_at_secs + 1, DEFAULT_TTL_SECS);
        assert_eq!(r, Err(VerifyError::BadSignature));
    }

    #[test]
    fn expired_signature_rejected() {
        let p = payload();
        let secret = b"secret-1";
        let s = sign(&p, secret);
        let now = p.issued_at_secs + DEFAULT_TTL_SECS + 1;
        assert_eq!(
            verify(&s, secret, None, now, DEFAULT_TTL_SECS),
            Err(VerifyError::Expired)
        );
    }

    #[test]
    fn rotation_overlap_old_url_verifies_under_previous_secret() {
        // URL minted under the old secret. Then rotate. Old URL
        // must still verify under (current, previous) within the
        // overlap window.
        let p = payload();
        let old = b"secret-old";
        let new = b"secret-new";
        let s = sign(&p, old);
        let now = p.issued_at_secs + 60; // 1 min after issue
                                         // After rotation: current = new, previous = old.
        let parsed = verify(&s, new, Some(old), now, DEFAULT_TTL_SECS).unwrap();
        assert_eq!(parsed, p);
        // After overlap closes (previous == None), the same URL is
        // rejected.
        let r = verify(&s, new, None, now, DEFAULT_TTL_SECS);
        assert_eq!(r, Err(VerifyError::BadSignature));
    }

    #[test]
    fn dedup_key_stable_across_rotation() {
        // Two URLs with the same nonce + kind under different
        // signing secrets must produce the same dedup_key. This
        // is the invariant from `API.md` § 4.
        let p = payload();
        let pid = &p.project_id;
        let dk1 = dedup_key(pid, EventKind::Impression, &p.nonce);
        let dk2 = dedup_key(pid, EventKind::Impression, &p.nonce);
        assert_eq!(dk1, dk2);
        let dk_click = dedup_key(pid, EventKind::Click, &p.nonce);
        assert_ne!(dk1, dk_click, "kind differentiates dedup slots");
    }

    #[test]
    fn dedup_key_per_project_isolated() {
        let nonce = [1u8; 8];
        let a = dedup_key("pj_a", EventKind::Impression, &nonce);
        let b = dedup_key("pj_b", EventKind::Impression, &nonce);
        assert_ne!(a, b, "different projects must not collide");
    }

    #[test]
    fn placement_id_hash_deterministic() {
        let h1 = placement_id_hash("pj_demo", "header");
        let h2 = placement_id_hash("pj_demo", "header");
        let h3 = placement_id_hash("pj_demo", "footer");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn malformed_input_rejected() {
        let secret = b"secret";
        let now = 1_700_000_000;
        assert_eq!(
            verify("not-a-signed-url", secret, None, now, 60),
            Err(VerifyError::Malformed)
        );
        assert_eq!(
            verify("AAAA.BBBB", secret, None, now, 60),
            Err(VerifyError::Malformed)
        );
    }
}
