import { createFileRoute } from '@tanstack/react-router';
import { Container, Stack, Text, Title } from '@mantine/core';

export const Route = createFileRoute('/')({
  component: PlaceholderHome,
});

function PlaceholderHome() {
  return (
    <Container size="md" py="xl">
      <Stack gap="md">
        <Title order={1}>Knievel Admin</Title>
        <Text c="dimmed">
          Phase 7.1 placeholder. Real routes land in 7.5+; auth lands in 7.4. See <code>UI.md</code>{' '}
          and <code>PHASES.md</code> for the plan.
        </Text>
      </Stack>
    </Container>
  );
}
