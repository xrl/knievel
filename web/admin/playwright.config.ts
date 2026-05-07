// Playwright config — Phase 7.10.
//
// Runs the e2e suite against `pnpm preview` (Vite's
// production-build server). The harness is wired here +
// committed; CI installs the browser binaries on demand
// (`npx playwright install` runs in nightly.yml only since
// the install adds ~250 MB and isn't worth bloating per-PR
// runs).
//
// Local: `pnpm exec playwright install chromium` once, then
// `pnpm test:e2e`.

import { defineConfig, devices } from '@playwright/test';

const BASE_URL = process.env.E2E_BASE_URL ?? 'http://127.0.0.1:4173';

export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['github'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL: BASE_URL,
    trace: 'on-first-retry',
  },
  webServer: {
    // Build is assumed to have run already (CI does
    // `pnpm build` before invoking this config); preview is
    // Vite's static server on :4173.
    command: 'pnpm preview --port 4173 --strictPort',
    url: BASE_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
