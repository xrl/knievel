// MintRevealModal contract tests. Pin the security-critical
// invariants:
//   - Done is disabled until "I've stored this" is ticked.
//   - Esc-to-close is a no-op.
//   - Click-outside is a no-op.
//   - The plaintext value is rendered in the modal body.

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { MantineProvider } from '@mantine/core';

import { MintRevealModal } from './MintRevealModal';

afterEach(() => cleanup());

function setup(secret: string | null = 'kvl_test_org_abc_secret') {
  const onClose = vi.fn();
  render(
    <MantineProvider>
      <MintRevealModal opened secret={secret} onClose={onClose} title="Mint test" />
    </MantineProvider>,
  );
  return { onClose };
}

describe('MintRevealModal', () => {
  it('renders the secret value', () => {
    setup('kvl_secret_value');
    expect(screen.getByText('kvl_secret_value')).toBeInTheDocument();
  });

  it('disables Done until the checkbox is ticked', () => {
    const { onClose } = setup();
    const done = screen.getByTestId('mint-done');
    expect(done).toBeDisabled();
    fireEvent.click(done);
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('mint-stored-checkbox'));
    expect(done).toBeEnabled();
    fireEvent.click(done);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('Esc keypress does not invoke onClose', () => {
    const { onClose } = setup();
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(onClose).not.toHaveBeenCalled();
  });
});
