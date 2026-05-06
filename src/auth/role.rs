//! Role enum.
//!
//! Five roles, ordered by privilege from `Reader` (least) up to
//! `OrgOwner` (most). The `Ord` derive uses declaration order, so
//! `Role::Reader < Role::Editor < Role::Admin < Role::OrgAdmin <
//! Role::OrgOwner`. Per `AUTH.md` "Authorization", an "at least
//! editor" check is `role >= Role::Editor`.
//!
//! Wire form is kebab-case (`reader`, `editor`, `admin`,
//! `org-admin`, `org-owner`) — matches the strings used in
//! `AUTH.md`, the JWT `knievel.role` claim, and the `api_tokens`
//! `role` column's CHECK constraint.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Reader,
    Editor,
    Admin,
    OrgAdmin,
    OrgOwner,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Reader => "reader",
            Role::Editor => "editor",
            Role::Admin => "admin",
            Role::OrgAdmin => "org-admin",
            Role::OrgOwner => "org-owner",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_matches_spec() {
        assert!(Role::Reader < Role::Editor);
        assert!(Role::Editor < Role::Admin);
        assert!(Role::Admin < Role::OrgAdmin);
        assert!(Role::OrgAdmin < Role::OrgOwner);
        // "At least editor" idiom — exercised at the boundary.
        assert!(Role::Admin >= Role::Editor);
        assert!(Role::OrgOwner >= Role::Editor);
    }

    #[test]
    fn kebab_case_serde_round_trip() {
        for (role, wire) in [
            (Role::Reader, "\"reader\""),
            (Role::Editor, "\"editor\""),
            (Role::Admin, "\"admin\""),
            (Role::OrgAdmin, "\"org-admin\""),
            (Role::OrgOwner, "\"org-owner\""),
        ] {
            assert_eq!(serde_json::to_string(&role).unwrap(), wire);
            assert_eq!(serde_json::from_str::<Role>(wire).unwrap(), role);
            assert_eq!(role.as_str(), wire.trim_matches('"'));
        }
    }

    #[test]
    fn unknown_role_rejected() {
        assert!(serde_json::from_str::<Role>("\"superadmin\"").is_err());
    }
}
