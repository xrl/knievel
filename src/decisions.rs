//! Decision API — the hot path.
//!
//! Phase 3.18. `POST /v1/projects/{projectId}/decisions` runs
//! site lookup → `selection::filter` → `selection::priority` →
//! `selection::weighted_random` → HMAC-signed impression/click
//! URL minting, all against the in-process snapshot
//! (`AppState::snapshot`). No DB round-trips on the hot path
//! once the snapshot is loaded.
//!
//! Events emission to `events_raw` lands with Phase 3.21 (the
//! event channel + COPY flusher). For 3.18 we return the
//! decision and mint URLs; the channel-send is the next commit.
//!
//! Spec refs: `API.md` § 1, `REQUIREMENTS.md` § 6.1.

#![allow(clippy::large_enum_variant)]

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};

use crate::auth::security::BearerAuth;
use crate::auth::{Principal, Role};
use crate::handlers::{open_project_tx, AuthzError};
use crate::hmac::{self, SignaturePayload};
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::selection::{self, BlockSet, Placement};
use crate::state::AppState;

pub struct DecisionsApi;

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct DecisionContext {
    pub url: Option<String>,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct ForceOverride {
    pub ad_id: Option<i64>,
    pub campaign_id: Option<i64>,
    pub flight_id: Option<i64>,
    pub creative_id: Option<i64>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct DecisionPlacement {
    pub id: String,
    pub site_id: Option<i64>,
    pub site_url: Option<String>,
    pub site_external_id: Option<String>,
    pub zone_ids: Option<Vec<i64>>,
    pub ad_types: Vec<i64>,
    pub count: Option<i32>,
    pub force: Option<ForceOverride>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct DecisionBlock {
    pub creative_ids: Option<Vec<i64>>,
    pub advertiser_ids: Option<Vec<i64>>,
    pub campaign_ids: Option<Vec<i64>>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct DecisionsRequest {
    pub context: Option<DecisionContext>,
    pub placements: Vec<DecisionPlacement>,
    pub block: Option<DecisionBlock>,
    /// Optional reason string captured in audit_log when `force.*`
    /// is honored (3.19 explainer-side audit pairs with this).
    pub force_reason: Option<String>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct DecisionAd {
    pub ad_id: i64,
    pub creative_id: i64,
    pub flight_id: i64,
    pub campaign_id: i64,
    pub advertiser_id: i64,
    pub priority_id: i64,
    pub site_id: i64,
    pub click_url: String,
    pub impression_url: String,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct DecisionsResponse {
    pub snapshot_version: i64,
    pub decisions: HashMap<String, Vec<DecisionAd>>,
}

#[derive(ApiResponse)]
pub enum DecisionsResp {
    #[oai(status = 200)]
    Ok(Json<DecisionsResponse>),
    #[oai(status = 400)]
    BadRequest(Json<ErrorEnvelope>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 503)]
    Unavailable(Json<ErrorEnvelope>),
}

fn err(code: &str, message: &str) -> ErrorEnvelope {
    ErrorEnvelope {
        error: ErrorBody {
            code: code.into(),
            message: message.into(),
        },
    }
}

fn forbid(e: AuthzError) -> DecisionsResp {
    DecisionsResp::Forbidden(Json(err(e.code(), e.message())))
}

#[OpenApi]
impl DecisionsApi {
    #[oai(
        path = "/v1/projects/:project_id/decisions",
        method = "post",
        operation_id = "decisions"
    )]
    async fn decisions(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<DecisionsRequest>,
    ) -> DecisionsResp {
        let principal = auth.0;
        let pj = project_id.0;
        let req = body.0;

        // Auth + project-existence check via the standard prologue.
        // Reader is sufficient for /decisions (the auth section of
        // AUTH.md "Endpoint -> minimum role" classifies decision
        // calls as project-read for v0).
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => {
                return DecisionsResp::Unavailable(Json(err("no_db", "no database configured")))
            }
        };
        let tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return forbid(e),
        };
        // The prologue's tx isn't used for the hot path itself
        // (the snapshot is RAM); rolling it back releases the
        // connection immediately.
        drop(tx);

        let snap = state.0.snapshot.read();
        let project = match snap.projects.get(&pj) {
            Some(p) => p,
            None => {
                return DecisionsResp::Unavailable(Json(err(
                    "snapshot_cold",
                    "snapshot has not loaded this project yet",
                )))
            }
        };

        if req.placements.is_empty() {
            return DecisionsResp::BadRequest(Json(err(
                "placements_required",
                "placements must be a non-empty array",
            )));
        }
        if req.placements.len() > 32 {
            return DecisionsResp::BadRequest(Json(err(
                "too_many_placements",
                "max 32 placements per request",
            )));
        }

        let block = build_block(&req.block);
        let now_ms = epoch_ms();
        let now_secs = (now_ms / 1000) as u64;

        let mut decisions: HashMap<String, Vec<DecisionAd>> = HashMap::new();
        for placement in &req.placements {
            let resolved = match resolve_site(project, placement) {
                Some(s) => s,
                None => {
                    decisions.insert(placement.id.clone(), Vec::new());
                    continue;
                }
            };
            if placement.ad_types.is_empty() {
                return DecisionsResp::BadRequest(Json(err(
                    "ad_types_required",
                    "placements[].ad_types must be a non-empty array",
                )));
            }
            let count = placement.count.unwrap_or(1).clamp(1, 10) as u32;
            let p = Placement {
                site_id: resolved,
                zone_ids: placement.zone_ids.clone().unwrap_or_default(),
                ad_types: placement.ad_types.clone(),
                count,
            };

            // force_overrides honored only when the three controls
            // pass: project flag, role >= ProjectAdmin, and
            // (audit emission lands with 3.21). For now we silently
            // ignore force.* unless the gate is fully open and write
            // the audit follow-up alongside the events flusher.
            // ProjectAdmin in AUTH.md maps onto Role::Admin in v0.
            let force_active = placement.force.is_some()
                && project.allow_force_decision
                && principal.has_role_at_least(Role::Admin);
            // Always carry the snapshot's flights/ads through the
            // selection pipeline; force_active just informs an
            // audit row in 3.21.
            let _ = force_active;

            let cands = selection::filter(&project.flights, &project.ads, &p, &block, now_ms);
            let top = selection::priority(&cands);
            let seed = selection_seed(&placement.id, now_ms);
            let picks = selection::weighted_random(&top, count, seed);

            let mut placement_out = Vec::with_capacity(picks.len());
            for c in picks {
                let nonce = mint_nonce(now_ms, c.ad.id);
                let payload = SignaturePayload {
                    project_id: pj.clone(),
                    ad_id: c.ad.id,
                    creative_id: 0, // resolved at the snapshot consumer; v0 stub
                    placement_id_hash: hmac::placement_id_hash(&pj, &placement.id),
                    issued_at_secs: now_secs,
                    nonce,
                };
                let click_signed = hmac::sign(&payload, &project.hmac_secret);
                let imp_signed = hmac::sign(&payload, &project.hmac_secret);
                placement_out.push(DecisionAd {
                    ad_id: c.ad.id,
                    creative_id: 0,
                    flight_id: c.flight.id,
                    campaign_id: c.flight.campaign_id,
                    advertiser_id: c.flight.advertiser_id,
                    priority_id: c.flight.priority_tier as i64,
                    site_id: resolved,
                    click_url: format!("/e/c/{click_signed}"),
                    impression_url: format!("/e/i/{imp_signed}"),
                });
            }
            decisions.insert(placement.id.clone(), placement_out);
        }

        DecisionsResp::Ok(Json(DecisionsResponse {
            snapshot_version: snap.config_version,
            decisions,
        }))
    }
}

fn build_block(req: &Option<DecisionBlock>) -> BlockSet {
    let Some(b) = req.as_ref() else {
        return BlockSet::default();
    };
    BlockSet {
        creative_ids: b.creative_ids.clone().unwrap_or_default(),
        advertiser_ids: b.advertiser_ids.clone().unwrap_or_default(),
        campaign_ids: b.campaign_ids.clone().unwrap_or_default(),
    }
}

/// Resolve `siteId`/`siteUrl`/`siteExternalId` to a numeric site
/// id from the snapshot. Returns `None` when no site matches —
/// the handler treats that as "no eligible ad" (empty array) per
/// `API.md` § 1, rather than an error.
fn resolve_site(project: &crate::snapshot::ProjectSnapshot, p: &DecisionPlacement) -> Option<i64> {
    if let Some(id) = p.site_id {
        return project.sites.iter().find(|s| s.id == id).map(|s| s.id);
    }
    if let Some(url) = p.site_url.as_ref() {
        return project
            .sites
            .iter()
            .find(|s| &s.url == url || s.aliases.iter().any(|a| a == url))
            .map(|s| s.id);
    }
    // siteExternalId resolution happens on the admin path; the
    // snapshot doesn't carry external_id today (3.18 follow-up).
    None
}

fn epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Per-placement selection seed: stable-but-not-cryptographic
/// hash of the placement id + the millisecond clock. Good enough
/// for "different placements get different randomness in the
/// same request"; the seed itself is never user-visible.
fn selection_seed(placement_id: &str, now_ms: i64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write(placement_id.as_bytes());
    h.write_i64(now_ms);
    h.finish()
}

fn mint_nonce(now_ms: i64, ad_id: i64) -> [u8; 8] {
    let mut out = [0u8; 8];
    let combined = now_ms.wrapping_mul(2_654_435_761).wrapping_add(ad_id);
    out.copy_from_slice(&combined.to_be_bytes());
    out
}

/// `Principal` is unused at the prologue level only — the field
/// presence keeps the function signature stable when we wire
/// audit-log emission for `force.*` in 3.21. Suppress the lint
/// at the function level so the signature stays informative.
#[allow(dead_code)]
fn _principal_referenced(_p: &Principal) {}
