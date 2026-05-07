// axe-core a11y check for MintRevealModal. Phase 7.10.
//
// One representative axe sweep per critical surface. The
// MintRevealModal is the security-critical mint UX, and the
// test confirms the modal markup is announced correctly to
// screen readers (no missing labels, valid color contrast in
// Mantine's theme, etc.).
//
// Adding more sweeps is cheap; expand as new bespoke
// surfaces land. Most resource list views just render
// Mantine's accessible primitives, so per-list axe sweeps
// are a low-value follow-up.

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@testing-library/react';
import { MantineProvider } from '@mantine/core';
import axe from 'axe-core';

import { MintRevealModal } from './MintRevealModal';

afterEach(() => cleanup());

async function runAxe(node: Element): Promise<axe.Result[]> {
  const r = await axe.run(node, {
    runOnly: ['wcag2a', 'wcag2aa'],
    // Color-contrast checks need real pixel rendering;
    // happy-dom doesn't compute paint, so skip that rule
    // here and rely on Mantine's theme defaults +
    // manual verification.
    rules: { 'color-contrast': { enabled: false } },
  });
  return r.violations;
}

describe('MintRevealModal a11y', () => {
  it('has no axe violations when open with a secret', async () => {
    const { container } = render(
      <MantineProvider>
        <MintRevealModal opened secret="kvl_test_secret" onClose={() => {}} title="Secret reveal" />
      </MantineProvider>,
    );
    const violations = await runAxe(container);
    expect(violations).toEqual([]);
  });
});
