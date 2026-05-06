//! Authentication and authorization primitives.
//!
//! Phase 3.2 lays down the building blocks (token format parsing,
//! argon2id hashing, the role enum, and the `Principal` shape) that
//! Phase 3.3 wires into a request extractor and the first handler.
//!
//! Spec refs:
//!   - `REQUIREMENTS.md` § 4.3 (auth modes overview)
//!   - `AUTH.md` (whole file)

// Module-level dead-code allow: parts of this module are populated
// in 3.2 but only consumed once 3.3 wires them through the
// extractor. Drop the attribute once consumers land.
#![allow(dead_code)]

pub mod opaque;
pub mod principal;
pub mod role;
pub mod security;

pub use principal::{Principal, Scope, TokenType};
pub use role::Role;
