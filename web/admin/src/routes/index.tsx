import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { Anchor, Code, Container, Group, Stack, Text, Title } from '@mantine/core';

import { apiClient } from '../api/client';
import { RequireAuth } from '../auth/RequireAuth';

export const Route = createFileRoute('/')({
  component: () => (
    <RequireAuth>
      <PlaceholderHome />
    </RequireAuth>
  ),
});

function PlaceholderHome() {
  // First end-to-end exercise of the typed client: hit
  // /v1/whoami and render the principal. Phase 7.5 replaces
  // this with the real org/project browser.
  const { data, error } = useQuery({
    queryKey: ['whoami'],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/whoami');
      if (error) throw error;
      return data;
    },
  });

  return (
    <Container size="md" py="xl">
      <Stack gap="md">
        <Group justify="space-between" align="center">
          <Title order={1}>Knievel Admin</Title>
          <Anchor href="/oidc/logout">Sign out</Anchor>
        </Group>
        <Text c="dimmed">
          Phase 7.4 placeholder. Real routes land in 7.5+. See <Code>UI.md</Code> and{' '}
          <Code>PHASES.md</Code> for the plan.
        </Text>
        {data && (
          <Stack gap="xs">
            <Text fw={600}>Signed in as</Text>
            <Code block>{JSON.stringify(data, null, 2)}</Code>
          </Stack>
        )}
        {error && <Text c="red">Failed to load whoami: {(error as Error).message}</Text>}
      </Stack>
    </Container>
  );
}
