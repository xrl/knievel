// Unit tests for the paste-token storage helpers. The OIDC
// path is integration-tested in 7.10 Playwright; here we just
// pin the contract that paste-token round-trips through
// sessionStorage and `clearPasteToken` truly clears.

import { afterEach, describe, expect, it } from 'vitest';

import {
  clearPasteToken,
  getCurrentBearer,
  getPasteToken,
  hasCredential,
  setPasteToken,
} from './session';

afterEach(() => {
  window.sessionStorage.clear();
});

describe('paste-token session', () => {
  it('round-trips through sessionStorage', () => {
    expect(getPasteToken()).toBeNull();
    setPasteToken('kvl_test_org_abc_secret');
    expect(getPasteToken()).toBe('kvl_test_org_abc_secret');
  });

  it('clears truly clears', () => {
    setPasteToken('kvl_test_org_abc_secret');
    clearPasteToken();
    expect(getPasteToken()).toBeNull();
  });

  it('hasCredential reflects paste-token presence', () => {
    expect(hasCredential()).toBe(false);
    setPasteToken('kvl_test_org_abc_secret');
    expect(hasCredential()).toBe(true);
    clearPasteToken();
    expect(hasCredential()).toBe(false);
  });

  it('getCurrentBearer returns paste-token when OIDC is absent', () => {
    setPasteToken('kvl_test_org_abc_secret');
    expect(getCurrentBearer()).toBe('kvl_test_org_abc_secret');
  });
});
