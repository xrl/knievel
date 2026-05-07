// axe-core a11y check for PasteTokenLogin. Phase 7.10
// follow-up.
//
// The fallback login form is the auth entry point when
// OIDC is disabled / unreachable. Locking in a WCAG 2 A/AA
// baseline keeps screen-reader users from getting wedged
// at sign-in.

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@testing-library/react';
import { MantineProvider } from '@mantine/core';
import axe from 'axe-core';

import { PasteTokenLogin } from './PasteTokenLogin';

afterEach(() => cleanup());

async function runAxe(node: Element): Promise<axe.Result[]> {
  const r = await axe.run(node, {
    runOnly: ['wcag2a', 'wcag2aa'],
    // happy-dom doesn't compute paint; rely on Mantine
    // theme defaults + manual verification for contrast.
    rules: { 'color-contrast': { enabled: false } },
  });
  return r.violations;
}

describe('PasteTokenLogin a11y', () => {
  it('has no axe violations in the resting state', async () => {
    const { container } = render(
      <MantineProvider>
        <PasteTokenLogin onSuccess={() => {}} />
      </MantineProvider>,
    );
    const violations = await runAxe(container);
    expect(violations).toEqual([]);
  });
});
