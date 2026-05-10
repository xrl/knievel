//! `knievel-cli` subcommand modules.
//!
//! Phase 4.2 introduced `seed-demo`. Subsequent work adds `admin`
//! (today: `create-org`; future: `mint-token`, `revoke-token`,
//! `list-orgs`, force.* audit dump). Future phases add `migrate`
//! (run migrations standalone) and `snapshot` (inspect the
//! in-memory snapshot for debug). Each subcommand lives in its
//! own file under this module.

pub mod admin;
pub mod seed_demo;
