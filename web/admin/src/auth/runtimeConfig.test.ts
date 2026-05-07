// Unit tests for the runtime config fetcher. Mocks `fetch`
// so we don't need a real API. Pins the safe-defaults
// behavior on network/parse failures since the SPA needs to
// boot to *something* (the paste-token form) even when the
// backend isn't reachable.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { _resetRuntimeConfigCacheForTests, loadRuntimeConfig, oidcEnabled } from './runtimeConfig';

beforeEach(() => {
  _resetRuntimeConfigCacheForTests();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('loadRuntimeConfig', () => {
  it('returns the API payload verbatim on 200', async () => {
    const payload = {
      oidc: {
        issuer: 'https://kc.example.com/realms/x',
        client_id: 'spa',
        scopes: ['openid', 'profile', 'knievel'],
        require_oidc: true,
      },
    };
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(JSON.stringify(payload), { status: 200 }),
    );
    const cfg = await loadRuntimeConfig();
    expect(cfg).toEqual(payload);
    expect(oidcEnabled(cfg)).toBe(true);
  });

  it('falls back to safe defaults on non-2xx', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('', { status: 503 }));
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const cfg = await loadRuntimeConfig();
    expect(cfg.oidc.issuer).toBe('');
    expect(cfg.oidc.client_id).toBe('');
    expect(oidcEnabled(cfg)).toBe(false);
  });

  it('falls back to safe defaults on network error', async () => {
    vi.spyOn(globalThis, 'fetch').mockRejectedValue(new Error('ECONNREFUSED'));
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const cfg = await loadRuntimeConfig();
    expect(cfg.oidc.issuer).toBe('');
    expect(oidcEnabled(cfg)).toBe(false);
  });

  it('caches the first call', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response('{"oidc":{"issuer":"","client_id":"","scopes":[],"require_oidc":false}}', {
        status: 200,
      }),
    );
    await loadRuntimeConfig();
    await loadRuntimeConfig();
    expect(fetchSpy).toHaveBeenCalledTimes(1);
  });
});
