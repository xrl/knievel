// Read-only JSON drawer used by the resource list views to
// inspect a single row's full payload (`UI.md` "IA / Read-
// only auditor views"). Per-resource detail routes can
// replace this when more structured detail UX exists; today
// the JSON drawer is the universal "what does this row
// look like" surface.

import { Drawer, Code, ScrollArea, Stack, Text, Title } from '@mantine/core';

interface Props<T> {
  opened: boolean;
  onClose: () => void;
  row: T | null;
  title?: string;
}

export function JsonDrawer<T>({ opened, onClose, row, title }: Props<T>) {
  return (
    <Drawer
      opened={opened}
      onClose={onClose}
      position="right"
      size="lg"
      title={<Title order={4}>{title ?? 'Details'}</Title>}
    >
      {row ? (
        <ScrollArea h="calc(100vh - 80px)">
          <Stack gap="xs">
            <Text size="sm" c="dimmed">
              Read-only view. Editing surfaces land in 7.7.
            </Text>
            <Code block>{JSON.stringify(row, null, 2)}</Code>
          </Stack>
        </ScrollArea>
      ) : null}
    </Drawer>
  );
}
