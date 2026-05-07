// Session — unified bearer accessor over OIDC + paste-token.
//
// The fetch wrapper calls `getCurrentBearer()` and doesn't
// care which flow produced the token. OIDC takes precedence
// when present (preferred path); the paste-token fallback is
// only consulted when OIDC isn't configured or the user
// hasn't completed OIDC login.

import { getOidcBearer } from './userManager';

const PASTE_TOKEN_KEY = 'knievel.paste_token';

export function getPasteToken(): string | null {
  try {
    return window.sessionStorage.getItem(PASTE_TOKEN_KEY);
  } catch {
    // sessionStorage can throw in private mode or with strict
    // CSP; treat as "no token" and let the UI prompt.
    return null;
  }
}

export function setPasteToken(token: string): void {
  window.sessionStorage.setItem(PASTE_TOKEN_KEY, token);
}

export function clearPasteToken(): void {
  window.sessionStorage.removeItem(PASTE_TOKEN_KEY);
}

/** Synchronous bearer accessor used by the fetch wrapper.
 *  Returns the OIDC access token if available, else the
 *  paste-token, else null (caller renders login). */
export function getCurrentBearer(): string | null {
  const oidc = getOidcBearer();
  if (oidc) return oidc;
  return getPasteToken();
}

/** True when *some* credential is available — either an
 *  unexpired OIDC user or a paste-token in sessionStorage.
 *  Used by RequireAuth's lightweight pre-check. */
export function hasCredential(): boolean {
  return getCurrentBearer() !== null;
}
