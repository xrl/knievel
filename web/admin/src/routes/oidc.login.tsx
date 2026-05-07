// `/oidc/login` — initiates `signinRedirect()` against
// Keycloak. Renders nothing visible; the user is bounced to
// the IdP and then back to `/oidc/callback`.

import { useEffect } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useAuth } from 'react-oidc-context';
import { Center, Loader } from '@mantine/core';

import { getRuntimeConfig, oidcEnabled } from '../auth/runtimeConfig';

interface SearchParams {
  return_to?: string;
}

export const Route = createFileRoute('/oidc/login')({
  validateSearch: (s: Record<string, unknown>): SearchParams => ({
    return_to: typeof s.return_to === 'string' ? s.return_to : undefined,
  }),
  component: OidcLogin,
});

function OidcLogin() {
  const auth = useAuth();
  const navigate = useNavigate();
  const search = Route.useSearch();
  const cfg = getRuntimeConfig();

  useEffect(() => {
    if (!cfg || !oidcEnabled(cfg)) {
      // OIDC isn't configured — redirect to paste-token login.
      navigate({ to: '/login', search, replace: true });
      return;
    }
    if (auth.isAuthenticated) {
      navigate({ to: search.return_to || '/', replace: true });
      return;
    }
    if (!auth.isLoading && !auth.activeNavigator) {
      void auth.signinRedirect({
        state: { return_to: search.return_to ?? '/' },
      });
    }
  }, [auth, cfg, navigate, search]);

  return (
    <Center mih={200}>
      <Loader />
    </Center>
  );
}
