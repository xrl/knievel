// Phase 7.10 e2e smoke. Verifies the SPA boots, fetches
// /admin/config.json, and renders the paste-token login
// when OIDC is disabled (the bootstrap shape).
//
// Doesn't exercise OIDC — that needs a running Keycloak
// fixture (out of scope for v0 e2e; covered by the
// AUTH.md § 7 manual verification block). The paste-token
// fallback is the realistic public-CI test surface.

import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  // Stub the runtime config endpoint so the SPA boots into
  // paste-token mode regardless of what API is reachable.
  await page.route('**/admin/config.json', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        oidc: {
          issuer: '',
          client_id: '',
          scopes: ['openid', 'profile', 'knievel'],
          require_oidc: false,
        },
      }),
    }),
  );
});

test('boots and renders the paste-token login', async ({ page }) => {
  await page.goto('/');
  await expect(page).toHaveURL(/\/login/);
  await expect(page.getByRole('heading', { name: /sign in/i })).toBeVisible();
  await expect(page.getByTestId('paste-token-input')).toBeVisible();
});

test('rejects an invalid token with a useful error', async ({ page }) => {
  // /v1/whoami returns 401 → the form should show an error.
  await page.route('**/v1/whoami', (route) =>
    route.fulfill({
      status: 401,
      contentType: 'application/json',
      body: JSON.stringify({
        error: { code: 'invalid_token', message: 'bad bearer' },
      }),
    }),
  );

  await page.goto('/');
  await page.getByTestId('paste-token-input').fill('kvl_test_garbage');
  await page.getByRole('button', { name: /sign in/i }).click();
  await expect(page.getByText(/Token rejected/i)).toBeVisible();
});
