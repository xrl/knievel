// UserManager singleton — one instance shared between the
// `react-oidc-context` <AuthProvider> and the API fetch
// wrapper. The fetch wrapper needs sync access to the access
// token (`UserManager.getUser()` is async); we cache the
// loaded user via the manager's events so reads stay sync.
//
// `UserManager` is null when OIDC isn't configured (empty
// issuer in /admin/config.json). In that case the paste-token
// fallback is the only route in.

import { UserManager, WebStorageStateStore, type User } from 'oidc-client-ts';

import type { RuntimeConfig } from './runtimeConfig';
import { oidcEnabled } from './runtimeConfig';

let manager: UserManager | null = null;
let cachedUser: User | null = null;

export function initUserManager(cfg: RuntimeConfig): UserManager | null {
  if (manager) return manager; // idempotent — boot may run twice in StrictMode
  if (!oidcEnabled(cfg)) return null;

  manager = new UserManager({
    authority: cfg.oidc.issuer,
    client_id: cfg.oidc.client_id,
    redirect_uri: `${window.location.origin}/admin/oidc/callback`,
    post_logout_redirect_uri: `${window.location.origin}/admin/`,
    response_type: 'code',
    scope: cfg.oidc.scopes.join(' '),
    // Persist tokens in sessionStorage — survives tab refresh,
    // dies on tab close. Acknowledged XSS tradeoff per UI.md
    // "Auth"; mitigated by short access-token TTL upstream.
    userStore: new WebStorageStateStore({ store: window.sessionStorage }),
    automaticSilentRenew: true,
    // Loaded as needed; keeps initial bundle smaller.
    loadUserInfo: false,
  });

  manager.events.addUserLoaded((user) => {
    cachedUser = user;
  });
  manager.events.addUserUnloaded(() => {
    cachedUser = null;
  });
  manager.events.addAccessTokenExpired(() => {
    cachedUser = null;
  });
  manager.events.addSilentRenewError((err) => {
    console.warn('OIDC silent renew failed:', err);
  });

  // Prime the cache from sessionStorage on boot — handles tab
  // refresh where the manager re-instantiates but the user is
  // already in storage.
  manager
    .getUser()
    .then((u) => {
      cachedUser = u;
    })
    .catch((err) => console.warn('OIDC getUser at init failed:', err));

  return manager;
}

export function getUserManager(): UserManager | null {
  return manager;
}

/** Sync access to the current OIDC bearer token. Null if OIDC
 *  is disabled, the user isn't signed in, or the token is
 *  expired (the cache is cleared on token-expired events). */
export function getOidcBearer(): string | null {
  if (!cachedUser || cachedUser.expired) return null;
  return cachedUser.access_token;
}

/** Test-only: reset the singleton between tests. */
export function _resetUserManagerForTests() {
  manager = null;
  cachedUser = null;
}
