// `/oidc/logout` — calls `signoutRedirect()` so logout
// invalidates the SSO session at Keycloak (Phase 7.9
// hardens the end_session integration). Also clears the
// paste-token from sessionStorage so a mixed-mode session
// doesn't leak past logout.

import { useEffect } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useAuth } from 'react-oidc-context';
import { Center, Loader } from '@mantine/core';

import { clearPasteToken } from '../auth/session';

export const Route = createFileRoute('/oidc/logout')({
  component: OidcLogout,
});

function OidcLogout() {
  const auth = useAuth();
  const navigate = useNavigate();

  useEffect(() => {
    clearPasteToken();
    if (auth.isAuthenticated) {
      void auth.signoutRedirect();
    } else {
      navigate({ to: '/', replace: true });
    }
  }, [auth, navigate]);

  return (
    <Center mih={200}>
      <Loader />
    </Center>
  );
}
