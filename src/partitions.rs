//! Partition manager for `events_raw`.
//!
//! Phase 3.23. Premakes 4 days of `events_raw_p<YYYY_MM_DD>`
//! partitions and detaches partitions older than the retention
//! window. Idempotent — calling twice in a row does nothing the
//! second time. Hangs off the leader (3.22) so exactly one
//! instance runs maintenance at a time.
//!
//! Spec: `REQUIREMENTS.md` § 7.4.

#![allow(dead_code)]

use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::PgPool;

use crate::leader::LeaderHandle;

/// Days of future partitions to keep premade. With hourly
/// maintenance ticks this means a leader outage of ≤ 4 days is
/// harmless (REQUIREMENTS.md § 7.4).
pub const PREMAKE_DAYS: i64 = 4;
/// Default retention. After this many days, the partition leaf
/// is detached (not dropped — operator decides when/whether to
/// drop). Configurable; spec default is 30 days.
pub const RETENTION_DAYS_DEFAULT: i64 = 30;
/// Maintenance tick interval.
pub const TICK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// One event-day in epoch seconds. UTC midnight.
fn day_start_secs(epoch_secs: i64) -> i64 {
    let day = 24 * 60 * 60;
    epoch_secs - epoch_secs.rem_euclid(day)
}

/// `events_raw_p<YYYY_MM_DD>` for the given UTC midnight.
fn leaf_name(day_start_secs: i64) -> String {
    // Manual y/m/d math from epoch — avoids pulling chrono in
    // for one format. Spec requires UTC.
    let (y, m, d) = ymd_from_epoch(day_start_secs);
    format!("events_raw_p{y:04}_{m:02}_{d:02}")
}

/// Convert epoch seconds (UTC midnight) → (year, month, day).
/// Algorithm: Howard Hinnant's days-from-epoch.
fn ymd_from_epoch(secs: i64) -> (i32, u32, u32) {
    let days = secs / 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5) + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// Single maintenance pass: premake the next `PREMAKE_DAYS` days
/// of partitions if missing, detach anything older than the
/// retention window. Idempotent.
pub async fn run_once(pool: &PgPool, retention_days: i64) -> Result<()> {
    let now_secs = epoch_secs_now();
    let today_start = day_start_secs(now_secs);
    for d in 0..PREMAKE_DAYS {
        let day_start = today_start + d * 86_400;
        let day_end = day_start + 86_400;
        let leaf = leaf_name(day_start);
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS knievel.{leaf}
                 PARTITION OF knievel.events_raw
                 FOR VALUES FROM (to_timestamp({day_start})) TO (to_timestamp({day_end}))"
        );
        sqlx::query(&sql)
            .execute(pool)
            .await
            .with_context(|| format!("create partition {leaf}"))?;
        // Each leaf needs RLS bound to the parent's policy. The
        // policy is inherited automatically by partition leaves;
        // ENABLE/FORCE flags are not, so re-apply them.
        let alter = format!(
            "ALTER TABLE knievel.{leaf} ENABLE ROW LEVEL SECURITY;
             ALTER TABLE knievel.{leaf} FORCE ROW LEVEL SECURITY;"
        );
        sqlx::query(&alter)
            .execute(pool)
            .await
            .with_context(|| format!("alter partition {leaf} for RLS"))?;
    }
    // Retention: detach anything that ends before
    // (now - retention_days). DETACH PARTITION CONCURRENTLY
    // requires the parent to be a partitioned table (it is).
    let cutoff = today_start - retention_days * 86_400;
    let cutoff_leaf = leaf_name(cutoff);
    // List partition leaves and detach those with names lexically
    // older than the cutoff. Naming convention is ascending,
    // so plain string compare is correct.
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT c.relname
           FROM pg_inherits i
           JOIN pg_class p ON p.oid = i.inhparent
           JOIN pg_class c ON c.oid = i.inhrelid
          WHERE p.relname = 'events_raw'
            AND c.relname LIKE 'events_raw_p%'",
    )
    .fetch_all(pool)
    .await
    .context("list partition leaves")?;
    for (leaf,) in rows {
        if leaf < cutoff_leaf && leaf.starts_with("events_raw_p") {
            let sql = format!("ALTER TABLE knievel.events_raw DETACH PARTITION knievel.{leaf}");
            if let Err(e) = sqlx::query(&sql).execute(pool).await {
                tracing::warn!(error = %e, leaf = %leaf, "detach partition failed");
            } else {
                tracing::info!(leaf = %leaf, "detached aged partition");
            }
        }
    }
    Ok(())
}

/// Hourly maintenance loop. Gated on the leader handle —
/// followers loop and check; only the leader actually runs the
/// maintenance pass. Records a tick on the leader handle so the
/// 3.22 watchdog stays satisfied.
pub fn spawn(
    pool: PgPool,
    leader: LeaderHandle,
    retention_days: i64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if !leader.is_leader() {
                continue;
            }
            match run_once(&pool, retention_days).await {
                Ok(()) => {
                    leader.record_tick().await;
                }
                Err(e) => {
                    tracing::error!(error = %e, "partition maintenance failed");
                }
            }
        }
    })
}

fn epoch_secs_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_start_is_idempotent_and_truncates() {
        let d = 86_400_i64; // 1970-01-02 midnight
        assert_eq!(day_start_secs(d), d);
        assert_eq!(day_start_secs(d + 1), d);
        assert_eq!(day_start_secs(d + 86_399), d);
        assert_eq!(day_start_secs(d + 86_400), d + 86_400);
    }

    #[test]
    fn leaf_name_format() {
        // 1970-01-01 midnight UTC = 0.
        let s = leaf_name(0);
        assert_eq!(s, "events_raw_p1970_01_01");
        // Day 1 = 1970-01-02.
        let s = leaf_name(86_400);
        assert_eq!(s, "events_raw_p1970_01_02");
    }

    #[test]
    fn leaf_name_lexical_order_matches_chronological() {
        // The retention sweep relies on lexical ordering of
        // names matching chronological order — pin it here.
        let a = leaf_name(0);
        let b = leaf_name(86_400 * 30);
        let c = leaf_name(86_400 * 365);
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn ymd_known_anchors() {
        assert_eq!(ymd_from_epoch(0), (1970, 1, 1));
        // 1970-01-02
        assert_eq!(ymd_from_epoch(86_400), (1970, 1, 2));
        // 2000-01-01 = 30 years * 365 + 7 leap days = 10957 days.
        assert_eq!(ymd_from_epoch(10_957 * 86_400), (2000, 1, 1));
        // 2000-02-29 is one day before 2000-03-01; just check
        // we transition cleanly via the leap day.
        let mar01_2000 = (10_957 + 31 + 29) * 86_400;
        assert_eq!(ymd_from_epoch(mar01_2000), (2000, 3, 1));
        let feb29_2000 = mar01_2000 - 86_400;
        assert_eq!(ymd_from_epoch(feb29_2000), (2000, 2, 29));
    }
}
