// Idempotency-Key helper. Knievel honors `Idempotency-Key`
// on every mutation per `API.md` "Idempotency / 24h replay
// window"; the SPA mints a fresh UUIDv4 per submit so a
// network retry on the same submit doesn't accidentally
// double-create. Different submits get different keys.
//
// `crypto.randomUUID()` is built into every browser we
// support (Edge 95+, Chrome 92+, Safari 15.4+, Firefox 95+);
// no dep needed.

export function newIdempotencyKey(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  // Fallback for ancient envs / non-browser unit-test
  // contexts without crypto.randomUUID. The shape doesn't
  // need to be a *real* UUID — knievel just needs an
  // opaque string ≤ 64 chars per the API contract.
  return `kvl-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}
