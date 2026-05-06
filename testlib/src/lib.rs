//! Test helpers for knievel's integration suite.
//!
//! Spec ref: `TESTING.md` § 5.1. Bridges the cases `sqlx::test`
//! doesn't cover — multi-connection setups for `LISTEN/NOTIFY`,
//! advisory-lock leader-election tests, and any test that needs
//! its own DB lifecycle rather than sqlx's auto-managed one.

pub mod db;
