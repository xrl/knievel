// Runtime config — fetched once at boot from the API at
// `/admin/config.json`. Keeps the bundle env-agnostic: one
// build artifact deploys to staging / prod / dev with different
// OIDC issuers + client IDs without rebuilding.
//
// Shape mirrors `src/admin_ui.rs::AdminUiConfigResponse` on the
// API side. Empty strings on `oidc.issuer` / `oidc.client_id`
// signal "OIDC disabled" — the SPA falls through to the paste-
// a-token fallback in that case (or shows the login screen with
// no OIDC button).

export interface RuntimeConfig {
  oidc: {
    issuer: string;
    client_id: string;
    scopes: string[];
    require_oidc: boolean;
  };
}

const DEFAULT_CONFIG: RuntimeConfig = {
  oidc: {
    issuer: '',
    client_id: '',
    scopes: ['openid', 'profile', 'knievel'],
    require_oidc: false,
  },
};

let cached: RuntimeConfig | null = null;

export async function loadRuntimeConfig(): Promise<RuntimeConfig> {
  if (cached) return cached;
  try {
    const resp = await fetch('/admin/config.json', { credentials: 'omit' });
    if (!resp.ok) {
      // API unreachable / endpoint missing → fall back to the
      // safe defaults so the SPA can still render the paste-
      // token form (operators may want to bring up the SPA
      // against an unconfigured cluster during bootstrap).
      console.warn(`/admin/config.json returned ${resp.status}; using defaults`);
      cached = DEFAULT_CONFIG;
      return cached;
    }
    const json = (await resp.json()) as RuntimeConfig;
    cached = json;
    return cached;
  } catch (err) {
    console.warn('failed to fetch /admin/config.json:', err);
    cached = DEFAULT_CONFIG;
    return cached;
  }
}

/** Test-only: reset the cache so each test starts clean. */
export function _resetRuntimeConfigCacheForTests() {
  cached = null;
}

/** Returns the cached config or null if not yet loaded. */
export function getRuntimeConfig(): RuntimeConfig | null {
  return cached;
}

export function oidcEnabled(cfg: RuntimeConfig): boolean {
  return cfg.oidc.issuer.length > 0 && cfg.oidc.client_id.length > 0;
}
