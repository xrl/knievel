// Read-only JSON drawer used by the resource list views to
// inspect a single row's full payload (`UI.md` "IA / Read-
// only auditor views"). Per-resource detail routes can
// replace this when more structured detail UX exists; today
// the JSON drawer is the universal "what does this row
// look like" surface.

import type { ReactNode } from 'react';
import { Code, Drawer, ScrollArea, Stack, Text, Title } from '@mantine/core';

interface Props<T> {
  opened: boolean;
  onClose: () => void;
  row: T | null;
  title?: string;
  /** Optional extras rendered above the JSON dump — used by
   *  e.g. creatives to slot in the image upload widget. */
  extras?: ReactNode;
}

export function JsonDrawer<T>({ opened, onClose, row, title, extras }: Props<T>) {
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
          <Stack gap="md">
            {extras}
            <Stack gap="xs">
              <Text size="sm" c="dimmed">
                Raw record. Field-level editing lands per resource as the editing surface (Phase
                7.7) rolls out beyond advertisers.
              </Text>
              <Code block>{JSON.stringify(row, null, 2)}</Code>
            </Stack>
          </Stack>
        </ScrollArea>
      ) : null}
    </Drawer>
  );
}
