// Auth-aware fetch wrapper used by every API call.
//
// Phase 7.4 (partial). Covers:
//
// - Bearer attachment: pulls the current credential via
//   `getCurrentBearer()` (OIDC access token preferred; paste-
//   token fallback) and injects `Authorization: Bearer <…>`.
// - X-Request-Id surfacing: every response (success or failure)
//   carries the API's request ID; we capture it so error
//   toasts can display it for support correlation
//   (`UI.md` "Error handling").
// - 401 handling: bare-bones for now — clears OIDC + paste-
//   token state and redirects to `/login` preserving the deep
//   link via `?return_to=`. Silent refresh + the full per-
//   status state machine land in Phase 7.4 part 3.
//
// Wraps `openapi-fetch` so consumers get fully-typed
// `client.GET('/v1/whoami', { ... })` calls against the
// generated bindings.

import createClient from 'openapi-fetch';
import type { Middleware } from 'openapi-fetch';

import type { paths } from './generated';
import { clearPasteToken, getCurrentBearer } from '../auth/session';

const REQUEST_ID_HEADER = 'x-request-id';

/** Last X-Request-Id we saw, captured on every response.
 *  Cleared when a fresh request fires. Consumers can read this
 *  from error handlers without threading it through every
 *  call site. */
let lastRequestId: string | null = null;

export function getLastRequestId(): string | null {
  return lastRequestId;
}

const authMiddleware: Middleware = {
  async onRequest({ request }) {
    const bearer = getCurrentBearer();
    if (bearer) request.headers.set('authorization', `Bearer ${bearer}`);
    return request;
  },
  async onResponse({ response }) {
    lastRequestId = response.headers.get(REQUEST_ID_HEADER);
    if (response.status === 401) {
      // Bearer rejected — clear paste-token state (the OIDC
      // user is cleared by the manager itself on its 401
      // signal). Don't navigate from here; that's a UX
      // decision the caller can make. Surfaces as a 401 to
      // TanStack Query which renders the login redirect
      // through RequireAuth.
      clearPasteToken();
    }
    return response;
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
