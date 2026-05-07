// Role-claim-driven UI gating. Phase 7.9.
//
// **Not a security boundary.** Knievel still enforces every
// authz check server-side. Hiding admin-only surfaces in
// the SPA is purely cosmetic — it keeps editor/reader users
// from clicking buttons that would 403 anyway.
//
// Role hierarchy from AUTH.md "Authorization":
//   reader < editor < admin < org-admin < org-owner
//
// The principal's role comes from /v1/whoami; a long
// staleTime in `useWhoami()` keeps this near-free.

import type { components } from '../api/generated';

export type Role = components['schemas']['WhoamiResponse']['role'];

const ORDER: Record<string, number> = {
  reader: 0,
  editor: 1,
  admin: 2,
  'org-admin': 3,
  'org-owner': 4,
};

/** True when `actual` is `min` or higher in the hierarchy.
 *  Unknown roles default to "below everything" so a
 *  malformed claim never accidentally grants permissions. */
export function hasRoleAtLeast(actual: string | undefined, min: Role): boolean {
  if (actual === undefined) return false;
  const a = ORDER[actual];
  const m = ORDER[min];
  if (a === undefined || m === undefined) return false;
  return a >= m;
}
