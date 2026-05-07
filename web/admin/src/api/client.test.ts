// Fetch-wrapper contract tests. Phase 7.10 follow-up.
//
// The `apiClient` is built once at module load with the
// `authMiddleware` registered. To test the middleware
// without rebuilding the client, we stub the auth/session
// and auth/userManager modules via `vi.mock()` and stub
// the global `fetch` via `spyFetch()`.
//
// Pinned invariants:
//   1. Authorization header carries the current bearer
//      from `getCurrentBearer()`.
//   2. X-Request-Id from every response is captured into
//      the module-level `lastRequestId`.
//   3. 401 with no OIDC user → `clearPasteToken()` runs
//      and the response surfaces unchanged.
//   4. 401 + OIDC user + GET → `signinSilent()` is
//      attempted; on success the request retries with the
//      fresh bearer.
//   5. 401 + OIDC user + POST → no retry (POST bodies may
//      already be consumed). Response surfaces unchanged.
//   6. The retry sentinel (`x-knievel-retry`) prevents an
//      infinite loop on persistent 401s.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// `vi.mock()` is hoisted to the top of the file before
// module-level `const`s execute, so we use `vi.hoisted()`
// to share the mock objects across the factory and the
// test bodies.
const { sessionMock, userManagerMock } = vi.hoisted(() => ({
  sessionMock: {
    getCurrentBearer: vi.fn<() => string | null>(() => null),
    clearPasteToken: vi.fn<() => void>(() => {}),
  },
  userManagerMock: {
    getUserManager: vi.fn<() => { signinSilent: () => Promise<void> } | null>(() => null),
    getOidcBearer: vi.fn<() => string | null>(() => null),
  },
}));

vi.mock('../auth/session', () => sessionMock);
vi.mock('../auth/userManager', () => userManagerMock);

import { apiClient, getLastRequestId } from './client';

beforeEach(() => {
  sessionMock.getCurrentBearer.mockReset();
  sessionMock.getCurrentBearer.mockReturnValue(null);
  sessionMock.clearPasteToken.mockReset();
  userManagerMock.getUserManager.mockReset();
  userManagerMock.getUserManager.mockReturnValue(null);
  userManagerMock.getOidcBearer.mockReset();
  userManagerMock.getOidcBearer.mockReturnValue(null);
});

// happy-dom binds `window.fetch` separately from
// `globalThis.fetch`, and `vi.spyOn(globalThis, 'fetch')`
// doesn't intercept the call openapi-fetch actually makes.
// Replace both bindings + the Window prototype's `fetch`
// so any of them route to the spy.
const origGlobalFetch = globalThis.fetch;
const origWindowFetch = typeof window !== 'undefined' ? window.fetch : undefined;

function spyFetch(): ReturnType<typeof vi.fn> {
  const mock = vi.fn();
  globalThis.fetch = mock as unknown as typeof fetch;
  if (typeof window !== 'undefined') {
    window.fetch = mock as unknown as typeof fetch;
  }
  return mock;
}

afterEach(() => {
  vi.restoreAllMocks();
  globalThis.fetch = origGlobalFetch;
  if (typeof window !== 'undefined' && origWindowFetch) {
    window.fetch = origWindowFetch;
  }
});

function jsonResponse(status: number, body: unknown, headers: Record<string, string> = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', ...headers },
  });
}

describe('apiClient auth middleware', () => {
  it('attaches Authorization: Bearer when a bearer is present', async () => {
    sessionMock.getCurrentBearer.mockReturnValue('kvl_test_org_abc_secret');
    const fetchSpy = spyFetch().mockResolvedValue(jsonResponse(200, { ok: true }));

    await apiClient.GET('/v1/whoami');

    const call = fetchSpy.mock.calls.at(-1);
    expect(call).toBeDefined();
    const request = call![0] as Request;
    expect(request.headers.get('authorization')).toBe('Bearer kvl_test_org_abc_secret');
  });

  it('omits Authorization when no bearer is present', async () => {
    sessionMock.getCurrentBearer.mockReturnValue(null);
    const fetchSpy = spyFetch().mockResolvedValue(jsonResponse(200, { ok: true }));

    await apiClient.GET('/v1/whoami');

    const request = fetchSpy.mock.calls.at(-1)![0] as Request;
    expect(request.headers.get('authorization')).toBeNull();
  });

  it('captures X-Request-Id from the response', async () => {
    spyFetch().mockResolvedValue(
      jsonResponse(200, { ok: true }, { 'x-request-id': 'req-abc-123' }),
    );

    await apiClient.GET('/v1/whoami');

    expect(getLastRequestId()).toBe('req-abc-123');
  });

  it('captures X-Request-Id even on 4xx errors', async () => {
    spyFetch().mockResolvedValue(
      jsonResponse(400, { error: { code: 'bad', message: 'no' } }, { 'x-request-id': 'req-err-9' }),
    );

    await apiClient.GET('/v1/whoami');

    expect(getLastRequestId()).toBe('req-err-9');
  });

  it('clears paste-token on 401 when no OIDC user is present', async () => {
    spyFetch().mockResolvedValue(jsonResponse(401, { error: { code: 'invalid_token' } }));

    await apiClient.GET('/v1/whoami');

    expect(sessionMock.clearPasteToken).toHaveBeenCalledTimes(1);
  });

  it('attempts silent refresh + retry on 401 GET when OIDC user is present', async () => {
    sessionMock.getCurrentBearer.mockImplementation(() => userManagerMock.getOidcBearer());
    userManagerMock.getOidcBearer
      .mockReturnValueOnce('OLD_TOKEN')
      .mockReturnValueOnce('OLD_TOKEN')
      .mockReturnValue('FRESH_TOKEN');
    const signinSilent = vi.fn().mockResolvedValue(undefined);
    userManagerMock.getUserManager.mockReturnValue({ signinSilent });

    const fetchSpy = spyFetch()
      .mockResolvedValueOnce(jsonResponse(401, { error: { code: 'invalid_token' } }))
      .mockResolvedValueOnce(jsonResponse(200, { whoami: 'ok' }));

    const result = await apiClient.GET('/v1/whoami');

    expect(signinSilent).toHaveBeenCalledTimes(1);
    expect(fetchSpy).toHaveBeenCalledTimes(2);
    const retryInit = fetchSpy.mock.calls[1][1] as RequestInit;
    const retryHeaders = new Headers(retryInit.headers);
    expect(retryHeaders.get('authorization')).toBe('Bearer FRESH_TOKEN');
    expect(retryHeaders.get('x-knievel-retry')).toBe('1');
    expect(result.response.status).toBe(200);
  });

  it('does NOT retry on 401 POST (request body may be consumed)', async () => {
    sessionMock.getCurrentBearer.mockReturnValue('OLD_TOKEN');
    userManagerMock.getOidcBearer.mockReturnValue('OLD_TOKEN');
    const signinSilent = vi.fn().mockResolvedValue(undefined);
    userManagerMock.getUserManager.mockReturnValue({ signinSilent });

    const fetchSpy = spyFetch().mockResolvedValueOnce(
      jsonResponse(401, { error: { code: 'invalid_token' } }),
    );

    await apiClient.POST('/v1/projects/{project_id}/advertisers', {
      params: { path: { project_id: 'pj_x' } },
      body: { name: 'Acme' },
    });

    expect(signinSilent).not.toHaveBeenCalled();
    expect(fetchSpy).toHaveBeenCalledTimes(1);
  });

  it('does not loop when the retry itself returns 401', async () => {
    sessionMock.getCurrentBearer.mockReturnValue('OLD_TOKEN');
    userManagerMock.getOidcBearer.mockReturnValue('OLD_TOKEN');
    const signinSilent = vi.fn().mockResolvedValue(undefined);
    userManagerMock.getUserManager.mockReturnValue({ signinSilent });

    const fetchSpy = spyFetch().mockResolvedValue(
      jsonResponse(401, { error: { code: 'invalid_token' } }),
    );

    const result = await apiClient.GET('/v1/whoami');

    expect(fetchSpy).toHaveBeenCalledTimes(2);
    expect(result.response.status).toBe(401);
  });

  it('clears paste-token when silent refresh throws', async () => {
    sessionMock.getCurrentBearer.mockReturnValue('OLD_TOKEN');
    userManagerMock.getOidcBearer.mockReturnValue('OLD_TOKEN');
    const signinSilent = vi.fn().mockRejectedValue(new Error('upstream IDP unreachable'));
    userManagerMock.getUserManager.mockReturnValue({ signinSilent });

    spyFetch().mockResolvedValue(jsonResponse(401, { error: { code: 'invalid_token' } }));

    await apiClient.GET('/v1/whoami');

    expect(signinSilent).toHaveBeenCalledTimes(1);
    expect(sessionMock.clearPasteToken).toHaveBeenCalledTimes(1);
  });
});
