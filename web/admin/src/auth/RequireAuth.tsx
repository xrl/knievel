// RequireAuth — route guard. Wraps protected routes and
// redirects unauthenticated users to the login screen.
//
// Two paths in:
//
// - OIDC: `useAuth()` from react-oidc-context says
//   `isAuthenticated`. If not, redirect to `/oidc/login`
//   preserving the original deep link via `?return_to=`.
// - Paste-token: no OIDC context, but a token in
//   sessionStorage counts as authenticated.
//
// Note: this is a UX guard, not a security boundary. Knievel
// enforces every authz check server-side; client-side gating
// is purely cosmetic (`UI.md` "Auth").

import type { ReactNode } from 'react';
import { useEffect } from 'react';
import { useNavigate, useRouterState } from '@tanstack/react-router';
import { useAuth } from 'react-oidc-context';
import { Center, Loader } from '@mantine/core';

import { hasCredential } from './session';
import { getRuntimeConfig } from './runtimeConfig';
import { oidcEnabled } from './runtimeConfig';

interface Props {
  children: ReactNode;
}

export function RequireAuth({ children }: Props) {
  const cfg = getRuntimeConfig();
  if (!cfg) {
    // Boot still loading runtime config; render a spinner.
    return (
      <Center mih={200}>
        <Loader />
      </Center>
    );
  }

  if (oidcEnabled(cfg)) {
    return <RequireOidcAuth>{children}</RequireOidcAuth>;
  }
  return <RequirePasteAuth>{children}</RequirePasteAuth>;
}

function RequireOidcAuth({ children }: Props) {
  const auth = useAuth();
  const navigate = useNavigate();
  const returnTo = useRouterState({ select: (s) => s.location.href });

  useEffect(() => {
    if (auth.isLoading || auth.activeNavigator) return;
    if (!auth.isAuthenticated && !hasCredential()) {
      navigate({
        to: '/oidc/login',
        search: { return_to: returnTo },
        replace: true,
      });
    }
  }, [auth.isLoading, auth.isAuthenticated, auth.activeNavigator, returnTo, navigate]);

  if (auth.isLoading || auth.activeNavigator) {
    return (
      <Center mih={200}>
        <Loader />
      </Center>
    );
  }
  if (!auth.isAuthenticated && !hasCredential()) return null;
  return <>{children}</>;
}

function RequirePasteAuth({ children }: Props) {
  const navigate = useNavigate();
  const returnTo = useRouterState({ select: (s) => s.location.href });

  useEffect(() => {
    if (!hasCredential()) {
      navigate({
        to: '/login',
        search: { return_to: returnTo },
        replace: true,
      });
    }
  }, [returnTo, navigate]);

  if (!hasCredential()) return null;
  return <>{children}</>;
}
