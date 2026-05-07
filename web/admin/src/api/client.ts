// Auth-aware fetch wrapper used by every API call.
//
// Phase 7.4. Covers:
//
// - Bearer attachment: pulls the current credential via
//   `getCurrentBearer()` (OIDC access token preferred; paste-
//   token fallback) and injects `Authorization: Bearer <…>`.
// - X-Request-Id surfacing: every response (success or failure)
//   carries the API's request ID; we capture it on a module-
//   level singleton so error toasts can surface it for support
//   correlation (`UI.md` "Error handling").
// - 401 handling: when an OIDC user exists, attempt
//   `signinSilent()` once and retry GETs with the fresh
//   token before giving up. POST/PATCH/etc. don't auto-retry
//   (their bodies may have been consumed); they fail with
//   401 and the next request picks up the refreshed token.
//   On silent-refresh failure (or paste-token mode), we
//   clear the paste-token; <RequireAuth> redirects to
//   `/oidc/login` or `/login` on the next render.
//
// Wraps `openapi-fetch` so consumers get fully-typed
// `client.GET('/v1/whoami')` calls against the generated
// bindings.

import createClient from 'openapi-fetch';
import type { Middleware } from 'openapi-fetch';

import type { paths } from './generated';
import { clearPasteToken, getCurrentBearer } from '../auth/session';
import { getOidcBearer, getUserManager } from '../auth/userManager';

const REQUEST_ID_HEADER = 'x-request-id';

/** Last X-Request-Id we saw, captured on every response. */
let lastRequestId: string | null = null;

export function getLastRequestId(): string | null {
  return lastRequestId;
}

/** Per-request flag — set on the original request, checked on
 *  the retry response so we don't loop on persistent 401s. */
const RETRY_SENTINEL = 'x-knievel-retry';

const authMiddleware: Middleware = {
  async onRequest({ request }) {
    const bearer = getCurrentBearer();
    if (bearer) request.headers.set('authorization', `Bearer ${bearer}`);
    return request;
  },
  async onResponse({ request, response }) {
    lastRequestId = response.headers.get(REQUEST_ID_HEADER);
    if (response.status !== 401) return response;
    if (request.headers.has(RETRY_SENTINEL)) return response;

    // Silent-refresh path: only meaningful when there's an
    // OIDC user. Paste-token bearers can't be refreshed.
    const manager = getUserManager();
    if (!manager || !getOidcBearer()) {
      clearPasteToken();
      return response;
    }

    // Retry only safe methods — POST/PATCH/etc. may have
    // already consumed their request body.
    if (request.method !== 'GET' && request.method !== 'HEAD') return response;

    try {
      await manager.signinSilent();
    } catch {
      clearPasteToken();
      return response;
    }
    const fresh = getOidcBearer();
    if (!fresh) return response;

    const retryHeaders = new Headers(request.headers);
    retryHeaders.set('authorization', `Bearer ${fresh}`);
    retryHeaders.set(RETRY_SENTINEL, '1');
    const retried = await fetch(request.url, {
      method: request.method,
      headers: retryHeaders,
      credentials: request.credentials,
      mode: request.mode,
      cache: request.cache,
      redirect: request.redirect,
      referrer: request.referrer,
      integrity: request.integrity,
    });
    lastRequestId = retried.headers.get(REQUEST_ID_HEADER);
    return retried;
  },
};

export const apiBaseUrl = (): string => {
  // Same-origin in prod (UI mounted at /admin/, API at /v1/).
  // Vite dev server proxies via vite.config.ts (proxy is opt-
  // in via env var; default same-origin still works against
  // a CORS-enabled API).
  return '';
};

export const apiClient = createClient<paths>({ baseUrl: apiBaseUrl() });
apiClient.use(authMiddleware);
