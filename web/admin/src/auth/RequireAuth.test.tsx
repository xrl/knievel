// `RequireAuth` paste-token path tests. Phase 7.10
// follow-up.
//
// Pinned invariants:
//   - Paste-token mode + credential present → renders
//     children.
//   - Paste-token mode + no credential → calls
//     navigate({ to: '/login', ...}) with return_to
//     pointing at the current pathname + search.
//   - Runtime config not yet loaded → renders the loading
//     spinner.
//
// The OIDC variant is exercised via Playwright (the SPA's
// integrated boot path stubs OIDC's metadata endpoint);
// per-component vitest coverage of `useAuth()` from
// react-oidc-context would require a heavier provider mock
// than is worth maintaining here.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { MantineProvider } from '@mantine/core';

const { runtimeMock, sessionMock, routerMock } = vi.hoisted(() => ({
  runtimeMock: {
    getRuntimeConfig: vi.fn<() => unknown>(() => null),
    oidcEnabled: vi.fn<() => boolean>(() => false),
  },
  sessionMock: {
    hasCredential: vi.fn<() => boolean>(() => false),
  },
  routerMock: {
    navigate: vi.fn(),
    location: { pathname: '/orgs/org_a/projects/pj_x', search: '' },
  },
}));

vi.mock('./runtimeConfig', () => runtimeMock);
vi.mock('./session', () => sessionMock);
vi.mock('@tanstack/react-router', () => ({
  useNavigate: () => routerMock.navigate,
  useRouterState: ({
    select,
  }: {
    select: (s: { location: typeof routerMock.location }) => unknown;
  }) => select({ location: routerMock.location }),
}));
vi.mock('react-oidc-context', () => ({
  // RequireAuth's paste path doesn't call useAuth, but
  // importing the module shouldn't blow up.
  useAuth: () => ({ isLoading: false, isAuthenticated: false }),
}));

import { RequireAuth } from './RequireAuth';

beforeEach(() => {
  runtimeMock.getRuntimeConfig.mockReset();
  runtimeMock.oidcEnabled.mockReset();
  runtimeMock.oidcEnabled.mockReturnValue(false);
  sessionMock.hasCredential.mockReset();
  sessionMock.hasCredential.mockReturnValue(false);
  routerMock.navigate.mockReset();
});

afterEach(() => cleanup());

function setRuntime(cfg: unknown) {
  runtimeMock.getRuntimeConfig.mockReturnValue(cfg);
}

describe('RequireAuth (paste-token mode)', () => {
  it('renders a loader while runtime config is unresolved', () => {
    setRuntime(null);
    render(
      <MantineProvider>
        <RequireAuth>
          <div data-testid="protected">protected</div>
        </RequireAuth>
      </MantineProvider>,
    );
    expect(screen.queryByTestId('protected')).toBeNull();
  });

  it('renders children when a paste-token credential is present', () => {
    setRuntime({ oidc: { issuer: '', client_id: '', scopes: [], require_oidc: false } });
    sessionMock.hasCredential.mockReturnValue(true);
    render(
      <MantineProvider>
        <RequireAuth>
          <div data-testid="protected">protected</div>
        </RequireAuth>
      </MantineProvider>,
    );
    expect(screen.getByTestId('protected')).toBeInTheDocument();
    expect(routerMock.navigate).not.toHaveBeenCalled();
  });

  it('redirects to /login with return_to when no credential is present', () => {
    setRuntime({ oidc: { issuer: '', client_id: '', scopes: [], require_oidc: false } });
    sessionMock.hasCredential.mockReturnValue(false);
    routerMock.location = {
      pathname: '/orgs/org_a/projects/pj_x/advertisers',
      search: '?q=acme',
    };

    render(
      <MantineProvider>
        <RequireAuth>
          <div data-testid="protected">protected</div>
        </RequireAuth>
      </MantineProvider>,
    );

    expect(screen.queryByTestId('protected')).toBeNull();
    expect(routerMock.navigate).toHaveBeenCalledWith({
      to: '/login',
      search: { return_to: '/orgs/org_a/projects/pj_x/advertisers?q=acme' },
      replace: true,
    });
  });
});
