// Smoke test — proves the harness works end-to-end (Vitest +
// happy-dom + Testing Library + Mantine). Real route tests
// land alongside the routes themselves in 7.5+.
import { render, screen } from '@testing-library/react';
import { MantineProvider } from '@mantine/core';
import { describe, expect, it } from 'vitest';

function Hello() {
  return <h1>Knievel Admin</h1>;
}

describe('test harness', () => {
  it('renders a Mantine-wrapped component', () => {
    render(
      <MantineProvider>
        <Hello />
      </MantineProvider>,
    );
    expect(screen.getByRole('heading', { name: 'Knievel Admin' })).toBeInTheDocument();
  });
});
