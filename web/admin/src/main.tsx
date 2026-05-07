// Entry point. Boot order:
//
// 1. Fetch /admin/config.json (runtime OIDC metadata, keeps
//    one bundle env-agnostic — UI.md "Auth / Runtime config").
// 2. Initialize the OIDC UserManager singleton from the
//    config (skipped when issuer is empty).
// 3. Mount React with the providers stack:
//      QueryClientProvider → MantineProvider →
//      Notifications → AuthProvider → RouterProvider.
// 4. Real route definitions live under `src/routes/` and are
//    aggregated into `routeTree.gen.ts` by the router plugin
//    at build/dev time (gitignored; regenerated on every run).

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MantineProvider } from '@mantine/core';
import { Notifications } from '@mantine/notifications';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider, createRouter } from '@tanstack/react-router';

import '@mantine/core/styles.css';
import '@mantine/notifications/styles.css';

import { routeTree } from './routeTree.gen';
import { AuthProvider } from './auth/AuthProvider';
import { IdleWarning } from './auth/IdleWarning';
import { initUserManager } from './auth/userManager';
import { loadRuntimeConfig } from './auth/runtimeConfig';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Admin views are audit-first; stale data is acceptable
      // up to a refetch. Tighten per-resource later if needed.
      staleTime: 30_000,
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
});

const router = createRouter({
  routeTree,
  defaultPreload: 'intent',
  context: { queryClient },
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}

async function boot() {
  const config = await loadRuntimeConfig();
  initUserManager(config);

  const rootEl = document.getElementById('root');
  if (!rootEl) throw new Error('#root not found in index.html');

  createRoot(rootEl).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <MantineProvider defaultColorScheme="auto">
          <Notifications />
          <AuthProvider config={config}>
            <IdleWarning />
            <RouterProvider router={router} />
          </AuthProvider>
        </MantineProvider>
      </QueryClientProvider>
    </StrictMode>,
  );
}

void boot();
