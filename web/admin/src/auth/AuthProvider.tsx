// Top-level auth provider. Conditional on whether OIDC is
// configured at runtime:
//
// - OIDC enabled (issuer + client_id present in
//   /admin/config.json) → wraps in `react-oidc-context`'s
//   <AuthProvider>. <RequireAuth> uses `useAuth()` from there.
// - OIDC disabled → renders children directly. The paste-
//   token fallback is the only route in; <RequireAuth>
//   detects this and falls back to `hasCredential()` from
//   `session.ts`.

import type { ReactNode } from 'react';
import { AuthProvider as OidcAuthProvider } from 'react-oidc-context';

import { oidcEnabled, type RuntimeConfig } from './runtimeConfig';
import { getUserManager, initUserManager } from './userManager';

interface Props {
  config: RuntimeConfig;
  children: ReactNode;
}

export function AuthProvider({ config, children }: Props) {
  if (!oidcEnabled(config)) return <>{children}</>;

  // Init the singleton — idempotent under StrictMode.
  initUserManager(config);
  const manager = getUserManager();
  if (!manager) return <>{children}</>;

  return <OidcAuthProvider userManager={manager}>{children}</OidcAuthProvider>;
}
