//! Decision API — the hot path.
//!
//! Phase 3.18. `POST /v1/projects/{projectId}/decisions` runs
//! site lookup → `selection::filter` → `selection::priority` →
//! `selection::weighted_random` → HMAC-signed impression/click
//! URL minting, all against the in-process snapshot
//! (`AppState::snapshot`). No DB round-trips on the hot path
//! once the snapshot is loaded.
//!
//! Phase 3.30 wires this into the events flusher
//! (`AppState::events`) so each pick produces one Decision
//! event row, and into `audit_log` for `force.*` honored
//! overrides per `API.md` § 1's three-control gate.
//!
//! Spec refs: `API.md` § 1, `REQUIREMENTS.md` § 6.1.

#![allow(clippy::large_enum_variant)]

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api_tags::ApiTags;
use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};
use sqlx::{Postgres, Transaction};

use crate::auth::security::BearerAuth;
use crate::auth::{Principal, Role};
use crate::events::{self, EventKind, EventSender, SendError};
use crate::handlers::{open_project_tx, AuthzError};
use crate::hmac::{self, SignaturePayload};
use crate::idempotency;
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::selection::{self, BlockSet, Candidate, Placement};
use crate::snapshot::ProjectSnapshot;
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

#[OpenApi(tag = "ApiTags::Decisions")]
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
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return forbid(e),
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

        // Snapshot grab happens before the force-gate check so
        // both reads work off the same point-in-time view.
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

        // Force-gate enforcement. Three project-side controls plus
        // the global kill switch (`API.md` § 1). When any
        // placement requests `force.*`, all four must pass; failure
        // is `403 force_disabled` regardless of which control
        // tripped (caller debugging info would just leak the gate's
        // state).
        let force_requested = req.placements.iter().any(|p| p.force.is_some());
        let force_active = if force_requested {
            match force_gate(
                &principal,
                project,
                state.0.decisions.force_overrides_enabled,
            ) {
                Ok(()) => true,
                Err(()) => {
                    return DecisionsResp::Forbidden(Json(err(
                        "force_disabled",
                        "force.* overrides not permitted for this principal/project",
                    )))
                }
            }
        } else {
            false
        };

        // When force is honored, write a single `force.honored`
        // audit row covering the request before computing picks.
        // The row carries a SHA-256 of the canonical request body
        // so we never persist secret material; the actor and the
        // optional `force_reason` round-trip through the row.
        if force_active {
            if let Err(resp) = audit_force_honored(&mut tx, &principal, &pj, &req).await {
                return resp;
            }
            if let Err(e) = tx.commit().await {
                tracing::error!(error = %e, "audit commit failed");
                return DecisionsResp::Unavailable(Json(err(
                    "db_error",
                    "could not commit audit row",
                )));
            }
        } else {
            // Fast path: drop the prologue's tx; nothing else
            // needs the connection.
            drop(tx);
        }

        let block = build_block(&req.block);
        let now_ms = epoch_ms();
        let now_secs = (now_ms / 1000) as u64;

        let mut decisions: HashMap<String, Vec<DecisionAd>> = HashMap::new();
        let mut events_to_send: Vec<events::Event> = Vec::new();
        let context = req.context.as_ref();
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

            let cands = selection::filter(&project.flights, &project.ads, &p, &block, now_ms);
            let top = selection::priority(&cands);
            let seed = selection_seed(&placement.id, now_ms);
            let mut picks: Vec<Candidate<'_>> = selection::weighted_random(&top, count, seed);

            // Force.adId substitution: replace the picks with the
            // forced ad if it exists in the snapshot. v0 honors
            // `force.adId` only — campaignId/flightId/creativeId
            // are accepted on the wire (so callers can stop
            // sending them once we wire them up) but ignored
            // during selection. Spec follow-up tracked in
            // `PHASES.md`.
            if force_active {
                if let Some(ad_id) = placement.force.as_ref().and_then(|f| f.ad_id) {
                    if let Some(forced) = forced_candidate(project, ad_id) {
                        picks = vec![forced];
                    } else {
                        picks.clear();
                    }
                }
            }

            let mut placement_out = Vec::with_capacity(picks.len());
            for c in &picks {
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
                events_to_send.push(decision_event(
                    &principal,
                    &pj,
                    placement,
                    resolved,
                    c,
                    &nonce,
                    snap.config_version,
                    now_ms,
                    context,
                ));
            }
            decisions.insert(placement.id.clone(), placement_out);
        }

        // Drain events into the channel after the response is
        // fully composed. Saturation fails fast at `503` per
        // `API.md` § 4 / `REQUIREMENTS.md` § 7.6.
        if let Some(sender) = state.0.events.as_ref() {
            if let Err(()) = drain_to_sender(sender, events_to_send) {
                return DecisionsResp::Unavailable(Json(err(
                    "event_channel_saturated",
                    "events flusher cannot keep up; retry with backoff",
                )));
            }
        }

        DecisionsResp::Ok(Json(DecisionsResponse {
            snapshot_version: snap.config_version,
            decisions,
        }))
    }
}

/// Three-control + kill-switch gate from `API.md` § 1.
/// Returns `Ok(())` only when every control passes; otherwise
/// `Err(())` so the caller emits `403 force_disabled`.
fn force_gate(
    principal: &Principal,
    project: &ProjectSnapshot,
    global_enabled: bool,
) -> Result<(), ()> {
    if !global_enabled {
        return Err(());
    }
    if !project.allow_force_decision {
        return Err(());
    }
    // "Project Admin or higher." `Role::Admin` is the v0 mapping.
    if !principal.has_role_at_least(Role::Admin) {
        return Err(());
    }
    Ok(())
}

/// Persist a `force.honored` audit row inside the prologue's
/// open transaction.
async fn audit_force_honored(
    tx: &mut Transaction<'_, Postgres>,
    principal: &Principal,
    project_id: &str,
    req: &DecisionsRequest,
) -> Result<(), DecisionsResp> {
    let payload_hash = match idempotency::body_hash(req) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "audit hash failed");
            return Err(DecisionsResp::Unavailable(Json(err(
                "internal_error",
                "could not hash audit payload",
            ))));
        }
    };
    if let Err(e) = sqlx::query(
        "INSERT INTO knievel.audit_log
            (org_id, project_id, actor, operation, payload_hash)
         VALUES ($1, $2, $3, 'force.honored', $4)",
    )
    .bind(&principal.org_id)
    .bind(project_id)
    .bind(&principal.actor_id)
    .bind(&payload_hash)
    .execute(&mut **tx)
    .await
    {
        tracing::error!(error = %e, "audit_log insert failed");
        return Err(DecisionsResp::Unavailable(Json(err(
            "db_error",
            "audit_log insert failed",
        ))));
    }
    Ok(())
}

/// Look up `ad_id` in the snapshot and synthesize a candidate
/// pointing at it. Returns `None` if the ad isn't in the
/// snapshot at all (caller treats that as "no eligible ad" and
/// returns an empty placement, which preserves the contract for
/// stale `force.adId` callers).
fn forced_candidate(project: &ProjectSnapshot, ad_id: i64) -> Option<Candidate<'_>> {
    let ad = project.ads.iter().find(|a| a.id == ad_id)?;
    let flight = project.flights.iter().find(|f| f.id == ad.flight_id)?;
    Some(Candidate { ad, flight })
}

/// Build a Decision event row for the COPY flusher.
#[allow(clippy::too_many_arguments)]
fn decision_event(
    principal: &Principal,
    project_id: &str,
    placement: &DecisionPlacement,
    site_id: i64,
    cand: &Candidate<'_>,
    nonce: &[u8; 8],
    snapshot_version: i64,
    now_ms: i64,
    context: Option<&DecisionContext>,
) -> events::Event {
    let zone_id = placement.zone_ids.as_ref().and_then(|z| z.first().copied());
    // Decision rows are not deduppable via the HMAC dedup_key
    // (Decisions aren't signed URLs); the events_raw uniqueness
    // on `(project_id, kind, dedup_key, ts)` accepts NULL for
    // dedup_key, so leaving it `None` skips dedup as intended.
    events::Event {
        ts_ms: now_ms,
        org_id: principal.org_id.clone(),
        project_id: project_id.to_string(),
        kind: EventKind::Decision,
        placement_id: Some(placement.id.clone()),
        site_id: Some(site_id),
        zone_id,
        ad_id: Some(cand.ad.id),
        creative_id: None,
        flight_id: Some(cand.flight.id),
        campaign_id: Some(cand.flight.campaign_id),
        advertiser_id: Some(cand.flight.advertiser_id),
        url: context.and_then(|c| c.url.clone()),
        referrer_host: context
            .and_then(|c| c.referrer.as_deref())
            .and_then(referrer_host),
        user_agent_hash: context.and_then(|c| c.user_agent.as_deref()).map(ua_hash),
        signature_nonce: Some(nonce.to_vec()),
        dedup_key: None,
        snapshot_version: Some(snapshot_version),
    }
}

/// Send each event into the bounded channel; bubble up the
/// first saturation as `Err(())`.
fn drain_to_sender(sender: &EventSender, batch: Vec<events::Event>) -> Result<(), ()> {
    for ev in batch {
        match sender.try_send(ev) {
            Ok(()) => {}
            Err(SendError::ChannelSaturated) => return Err(()),
            Err(SendError::FlusherDown) => {
                // Flusher's gone; treat as saturation for the
                // caller — the cluster is in degraded mode and
                // backoff is the right behavior.
                return Err(());
            }
        }
    }
    Ok(())
}

fn referrer_host(referrer: &str) -> Option<String> {
    // Tiny URL-host extractor — pulls the host between "://" and
    // the next "/" or "?". Avoids pulling in `url` for one field.
    let after_scheme = referrer
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(referrer);
    let host = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn ua_hash(ua: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    Sha256::digest(ua.as_bytes()).to_vec()
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

// ----------------------------------------------------------------
// Phase 3.19 — Decision explainer.
//
// `POST /v1/projects/{projectId}/decisions:explain` accepts the
// same request body as `:decisions` and returns the same
// `decisions` payload plus a per-placement `explanation` block
// listing every candidate ad and the rules that were applied.
// No event is recorded; impression / click URLs returned are
// dummy placeholders. Same auth as `decisions`.

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct ExplainEvaluation {
    pub rule: String,
    pub result: String,
    pub detail: Option<String>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct ExplainCandidate {
    pub ad_id: i64,
    pub creative_id: i64,
    pub flight_id: i64,
    pub campaign_id: i64,
    pub advertiser_id: i64,
    pub weight: i32,
    pub evaluation: Vec<ExplainEvaluation>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct ExplainPlacement {
    pub priority_tier: Option<i32>,
    pub selected_ad_id: Option<i64>,
    pub candidates: Vec<ExplainCandidate>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct ExplainResponse {
    pub snapshot_version: i64,
    pub decisions: HashMap<String, Vec<DecisionAd>>,
    pub explanation: HashMap<String, ExplainPlacement>,
}

#[derive(ApiResponse)]
pub enum ExplainResp {
    #[oai(status = 200)]
    Ok(Json<ExplainResponse>),
    #[oai(status = 400)]
    BadRequest(Json<ErrorEnvelope>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 503)]
    Unavailable(Json<ErrorEnvelope>),
}

pub struct ExplainApi;

#[OpenApi(tag = "ApiTags::Explain")]
impl ExplainApi {
    #[oai(
        path = "/v1/projects/:project_id/decisions:explain",
        method = "post",
        operation_id = "decisionsExplain"
    )]
    async fn explain(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<DecisionsRequest>,
    ) -> ExplainResp {
        let principal = auth.0;
        let pj = project_id.0;
        let req = body.0;

        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return ExplainResp::Unavailable(Json(err("no_db", "no database configured"))),
        };
        let tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return ExplainResp::Forbidden(Json(err(e.code(), e.message()))),
        };
        drop(tx);

        let snap = state.0.snapshot.read();
        let project = match snap.projects.get(&pj) {
            Some(p) => p,
            None => {
                return ExplainResp::Unavailable(Json(err(
                    "snapshot_cold",
                    "snapshot has not loaded this project yet",
                )))
            }
        };

        if req.placements.is_empty() {
            return ExplainResp::BadRequest(Json(err(
                "placements_required",
                "placements must be a non-empty array",
            )));
        }
        if req.placements.len() > 32 {
            return ExplainResp::BadRequest(Json(err(
                "too_many_placements",
                "max 32 placements per request",
            )));
        }

        // Same force gate as `:decisions`. The explainer doesn't
        // emit events or audit rows (it's read-only debug), but
        // the gate is a safety property — silently letting the
        // explainer reveal "what would force.* return?" without
        // the controls would defeat the role check.
        let force_requested = req.placements.iter().any(|p| p.force.is_some());
        let force_active = if force_requested {
            match force_gate(
                &principal,
                project,
                state.0.decisions.force_overrides_enabled,
            ) {
                Ok(()) => true,
                Err(()) => {
                    return ExplainResp::Forbidden(Json(err(
                        "force_disabled",
                        "force.* overrides not permitted for this principal/project",
                    )))
                }
            }
        } else {
            false
        };

        let block = build_block(&req.block);
        let now_ms = epoch_ms();
        let mut decisions: HashMap<String, Vec<DecisionAd>> = HashMap::new();
        let mut explanation: HashMap<String, ExplainPlacement> = HashMap::new();

        for placement in &req.placements {
            let mut candidates_out: Vec<ExplainCandidate> = Vec::new();
            let resolved = resolve_site(project, placement);
            // Walk every ad in the snapshot; classify each rule.
            for ad in &project.ads {
                let Some(flight) = project.flights.iter().find(|f| f.id == ad.flight_id) else {
                    continue;
                };
                let mut eval = Vec::new();
                let mut all_pass = true;

                if !flight.is_active {
                    eval.push(ExplainEvaluation {
                        rule: "flight_active".into(),
                        result: "fail".into(),
                        detail: Some("flight.is_active = false".into()),
                    });
                    all_pass = false;
                } else {
                    eval.push(ExplainEvaluation {
                        rule: "flight_active".into(),
                        result: "pass".into(),
                        detail: None,
                    });
                }
                if !ad.is_active {
                    eval.push(ExplainEvaluation {
                        rule: "ad_active".into(),
                        result: "fail".into(),
                        detail: Some("ad.is_active = false".into()),
                    });
                    all_pass = false;
                } else {
                    eval.push(ExplainEvaluation {
                        rule: "ad_active".into(),
                        result: "pass".into(),
                        detail: None,
                    });
                }
                if let Some(s) = resolved {
                    let pass = flight.site_ids.is_empty() || flight.site_ids.contains(&s);
                    eval.push(ExplainEvaluation {
                        rule: "site_match".into(),
                        result: if pass { "pass".into() } else { "fail".into() },
                        detail: if pass {
                            None
                        } else {
                            Some(format!(
                                "site_id {s} not in flight.site_ids {:?}",
                                flight.site_ids
                            ))
                        },
                    });
                    all_pass &= pass;
                } else {
                    eval.push(ExplainEvaluation {
                        rule: "site_match".into(),
                        result: "fail".into(),
                        detail: Some("placement site did not resolve".into()),
                    });
                    all_pass = false;
                }
                let at_match = flight.ad_types.is_empty()
                    || placement
                        .ad_types
                        .iter()
                        .any(|t| flight.ad_types.contains(t));
                eval.push(ExplainEvaluation {
                    rule: "ad_type_match".into(),
                    result: if at_match {
                        "pass".into()
                    } else {
                        "fail".into()
                    },
                    detail: if at_match {
                        None
                    } else {
                        Some(format!(
                            "no overlap with flight.ad_types {:?}",
                            flight.ad_types
                        ))
                    },
                });
                all_pass &= at_match;

                let block_drop = block.advertiser_ids.contains(&flight.advertiser_id)
                    || block.campaign_ids.contains(&flight.campaign_id);
                eval.push(ExplainEvaluation {
                    rule: "block_advertiser_or_campaign".into(),
                    result: if block_drop {
                        "fail".into()
                    } else {
                        "pass".into()
                    },
                    detail: if block_drop {
                        Some("excluded by block.*".into())
                    } else {
                        None
                    },
                });
                all_pass &= !block_drop;

                let _ = all_pass;
                candidates_out.push(ExplainCandidate {
                    ad_id: ad.id,
                    creative_id: 0,
                    flight_id: flight.id,
                    campaign_id: flight.campaign_id,
                    advertiser_id: flight.advertiser_id,
                    weight: ad.weight,
                    evaluation: eval,
                });
            }

            // Now run the actual selection so the explanation
            // matches what /decisions would have done.
            if let Some(s) = resolved {
                if !placement.ad_types.is_empty() {
                    let count = placement.count.unwrap_or(1).clamp(1, 10) as u32;
                    let p = Placement {
                        site_id: s,
                        zone_ids: placement.zone_ids.clone().unwrap_or_default(),
                        ad_types: placement.ad_types.clone(),
                        count,
                    };
                    let cands =
                        selection::filter(&project.flights, &project.ads, &p, &block, now_ms);
                    let top = selection::priority(&cands);
                    let seed = selection_seed(&placement.id, now_ms);
                    let mut picks: Vec<Candidate<'_>> =
                        selection::weighted_random(&top, count, seed);
                    if force_active {
                        if let Some(ad_id) = placement.force.as_ref().and_then(|f| f.ad_id) {
                            if let Some(forced) = forced_candidate(project, ad_id) {
                                picks = vec![forced];
                            } else {
                                picks.clear();
                            }
                        }
                    }
                    let selected_ad = picks.first().map(|c| c.ad.id);
                    let tier = top.first().map(|c| c.flight.priority_tier);
                    // Mark the selected ad in the explanation.
                    if let Some(sel) = selected_ad {
                        for c in candidates_out.iter_mut() {
                            if c.ad_id == sel {
                                c.evaluation.push(ExplainEvaluation {
                                    rule: "weighted_random".into(),
                                    result: "selected".into(),
                                    detail: None,
                                });
                            }
                        }
                    }
                    let placement_decisions: Vec<DecisionAd> = picks
                        .iter()
                        .map(|c| DecisionAd {
                            ad_id: c.ad.id,
                            creative_id: 0,
                            flight_id: c.flight.id,
                            campaign_id: c.flight.campaign_id,
                            advertiser_id: c.flight.advertiser_id,
                            priority_id: c.flight.priority_tier as i64,
                            site_id: s,
                            // Per spec: explainer URLs are
                            // dummy placeholders, marked as such
                            // so callers don't accidentally serve
                            // them.
                            click_url: "/e/c/__explain_dummy__".into(),
                            impression_url: "/e/i/__explain_dummy__".into(),
                        })
                        .collect();
                    decisions.insert(placement.id.clone(), placement_decisions);
                    explanation.insert(
                        placement.id.clone(),
                        ExplainPlacement {
                            priority_tier: tier,
                            selected_ad_id: selected_ad,
                            candidates: candidates_out,
                        },
                    );
                    continue;
                }
            }
            decisions.insert(placement.id.clone(), Vec::new());
            explanation.insert(
                placement.id.clone(),
                ExplainPlacement {
                    priority_tier: None,
                    selected_ad_id: None,
                    candidates: candidates_out,
                },
            );
        }

        ExplainResp::Ok(Json(ExplainResponse {
            snapshot_version: snap.config_version,
            decisions,
            explanation,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{Principal, Role, Scope, TokenType};
    use crate::selection::{Ad as SelAd, Flight as SelFlight};

    fn principal_at(role: Role) -> Principal {
        Principal {
            token_type: TokenType::Opaque,
            scope: Scope::Org,
            org_id: "org_a".into(),
            project_id: None,
            role,
            actor_id: "tok_test".into(),
        }
    }

    fn project_with(allow_force: bool) -> ProjectSnapshot {
        ProjectSnapshot {
            project_id: "pj_a".into(),
            org_id_for_event: "org_a".into(),
            allow_force_decision: allow_force,
            ..ProjectSnapshot::default()
        }
    }

    #[test]
    fn force_gate_passes_only_when_all_three_controls_open() {
        let admin = principal_at(Role::Admin);
        let editor = principal_at(Role::Editor);
        let p_open = project_with(true);
        let p_closed = project_with(false);

        assert!(force_gate(&admin, &p_open, true).is_ok());
        // Global kill switch off → reject.
        assert!(force_gate(&admin, &p_open, false).is_err());
        // Per-project flag off → reject.
        assert!(force_gate(&admin, &p_closed, true).is_err());
        // Role too low → reject.
        assert!(force_gate(&editor, &p_open, true).is_err());
    }

    #[test]
    fn forced_candidate_returns_ad_when_present() {
        let mut p = project_with(true);
        p.flights.push(SelFlight {
            id: 10,
            campaign_id: 1,
            advertiser_id: 1,
            priority_tier: 5,
            start_ms: None,
            end_ms: None,
            site_ids: vec![],
            zone_ids: vec![],
            ad_types: vec![],
            is_active: true,
        });
        p.ads.push(SelAd {
            id: 99,
            flight_id: 10,
            weight: 1,
            is_active: true,
        });
        let c = forced_candidate(&p, 99).expect("forced ad found");
        assert_eq!(c.ad.id, 99);
        assert_eq!(c.flight.id, 10);
        assert!(forced_candidate(&p, 7).is_none());
    }

    #[test]
    fn referrer_host_strips_scheme_and_path() {
        assert_eq!(
            referrer_host("https://www.google.com/search?q=hi"),
            Some("www.google.com".into())
        );
        assert_eq!(
            referrer_host("http://example.com"),
            Some("example.com".into())
        );
        // Already host-only
        assert_eq!(referrer_host("acme.test/x"), Some("acme.test".into()));
        assert_eq!(referrer_host(""), None);
    }
}
