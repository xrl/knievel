//! `knievel-cli` subcommand modules.
//!
//! Phase 4.2. Today the CLI exposes a single subcommand,
//! `seed-demo`. Future phases add `admin` (token mint/revoke,
//! force.* audit dump), `migrate` (run migrations standalone),
//! and `snapshot` (inspect the in-memory snapshot for debug).
//! Each subcommand lives in its own file under this module.

pub mod seed_demo;
