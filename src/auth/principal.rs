//! `Principal` — the validated identity of a request.
//!
//! Built by the auth extractor (Phase 3.3 lands the opaque-token
//! path; Phase 3.26 adds JWTs). Once constructed, downstream
//! handlers consume the principal uniformly regardless of how the
//! request was authenticated — same role enum, same scope semantics
//! (`AUTH.md` "Authorization").

use crate::auth::Role;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Opaque,
    Jwt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Org,
    Project,
}

#[derive(Debug, Clone)]
pub struct Principal {
    pub token_type: TokenType,
    pub scope: Scope,
    pub org_id: String,
    /// `Some` only for project-scoped tokens. Org-scoped principals
    /// resolve the project from the request path at handler entry.
    pub project_id: Option<String>,
    pub role: Role,
}

impl Principal {
    /// True if the principal's role is at least `min`. Wraps the
    /// `Ord` impl on `Role` so handler-side checks read like prose:
    /// `if !principal.has_role_at_least(Role::Editor) { ... }`.
    pub fn has_role_at_least(&self, min: Role) -> bool {
        self.role >= min
    }
}
