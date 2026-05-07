import { Outlet, createRootRouteWithContext } from '@tanstack/react-router';
import type { QueryClient } from '@tanstack/react-query';
import { AppShell } from '@mantine/core';

interface RouterContext {
  queryClient: QueryClient;
}

export const Route = createRootRouteWithContext<RouterContext>()({
  component: RootLayout,
});

function RootLayout() {
  return (
    <AppShell padding="md">
      <AppShell.Main>
        <Outlet />
      </AppShell.Main>
    </AppShell>
  );
}
