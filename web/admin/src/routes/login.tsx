// `/login` — paste-a-token fallback. Reachable when:
//
// - Runtime config has no OIDC issuer (bootstrap / dev), OR
// - OIDC is configured but `require_oidc: false` and the user
//   chose the fallback link, OR
// - Keycloak is unreachable (DR scenario).

import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { Anchor, Stack, Text } from '@mantine/core';

import { PasteTokenLogin } from '../auth/PasteTokenLogin';
import { getRuntimeConfig, oidcEnabled } from '../auth/runtimeConfig';

interface SearchParams {
  return_to?: string;
}

export const Route = createFileRoute('/login')({
  validateSearch: (s: Record<string, unknown>): SearchParams => ({
    return_to: typeof s.return_to === 'string' ? s.return_to : undefined,
  }),
  component: LoginPage,
});

function LoginPage() {
  const navigate = useNavigate();
  const search = Route.useSearch();
  const cfg = getRuntimeConfig();

  return (
    <Stack gap="md">
      <PasteTokenLogin onSuccess={() => navigate({ to: search.return_to || '/', replace: true })} />
      {cfg && oidcEnabled(cfg) && !cfg.oidc.require_oidc && (
        <Text size="sm" c="dimmed" ta="center">
          Or{' '}
          <Anchor onClick={() => navigate({ to: '/oidc/login', search })}>
            sign in with single sign-on
          </Anchor>
          .
        </Text>
      )}
    </Stack>
  );
}
