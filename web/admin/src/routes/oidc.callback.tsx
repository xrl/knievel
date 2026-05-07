// `/oidc/callback` — completes the PKCE flow.
// react-oidc-context handles `signinRedirectCallback()`
// automatically when the URL contains `?code=...&state=...`
// (configured via the `<AuthProvider>`'s default behavior).
// We just wait for `auth.isAuthenticated` to flip true and
// redirect to the deep link from `state.return_to`.

import { useEffect } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useAuth } from 'react-oidc-context';
import { Alert, Center, Container, Loader, Stack, Text, Title } from '@mantine/core';

export const Route = createFileRoute('/oidc/callback')({
  component: OidcCallback,
});

function OidcCallback() {
  const auth = useAuth();
  const navigate = useNavigate();

  useEffect(() => {
    if (auth.isLoading || auth.activeNavigator) return;
    if (auth.isAuthenticated) {
      const returnTo = (auth.user?.state as { return_to?: string } | undefined)?.return_to ?? '/';
      navigate({ to: returnTo, replace: true });
    }
  }, [auth.isLoading, auth.isAuthenticated, auth.activeNavigator, auth.user, navigate]);

  if (auth.error) {
    return (
      <Container size="sm" py="xl">
        <Stack gap="md">
          <Title order={2}>Sign-in failed</Title>
          <Alert color="red" variant="light">
            {auth.error.message}
          </Alert>
          <Text size="sm" c="dimmed">
            Try again, or contact your administrator if the problem persists.
          </Text>
        </Stack>
      </Container>
    );
  }

  return (
    <Center mih={200}>
      <Loader />
    </Center>
  );
}
