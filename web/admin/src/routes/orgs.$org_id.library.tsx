// `/orgs/{org_id}/library` — Ad Library items (org-scoped).
// Phase 7.6.

import { useEffect, useState } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { Badge, Container, Stack, Title } from '@mantine/core';

import { apiClient } from '../api/client';
import type { components } from '../api/generated';
import { notifyApiError } from '../api/errors';
import { RequireAuth } from '../auth/RequireAuth';
import { DataTable } from '../components/DataTable';
import { JsonDrawer } from '../components/JsonDrawer';

type LibraryItem = components['schemas']['AdLibraryItem'];

export const Route = createFileRoute('/orgs/$org_id/library')({
  component: () => (
    <RequireAuth>
      <Library />
    </RequireAuth>
  ),
});

function Library() {
  const { org_id } = Route.useParams();
  const { data, isLoading, error } = useQuery({
    queryKey: ['ad-library', org_id],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/orgs/{org_id}/ad-library/items', {
        params: { path: { org_id } },
      });
      if (error) throw error;
      return data;
    },
  });
  const [selected, setSelected] = useState<LibraryItem | null>(null);
  useEffect(() => {
    if (error) notifyApiError(error);
  }, [error]);

  return (
    <Container size="lg" py="xl">
      <Stack gap="md">
        <Title order={1}>Ad library</Title>
        <DataTable<LibraryItem>
          description="Org-scoped reusable creative assets. Project Ads can reference these by ad_library_item_id."
          loading={isLoading}
          error={error}
          items={data?.items ?? []}
          rowKey={(r) => r.id}
          onRowClick={(r) => setSelected(r)}
          columns={[
            { key: 'name', label: 'Name' },
            { key: 'id', label: 'ID', render: (v) => <code>{String(v)}</code> },
            {
              key: 'external_id',
              label: 'External ID',
              render: (v) => (v ? <code>{String(v)}</code> : '—'),
            },
            { key: 'kind', label: 'Kind', render: (v) => <Badge>{String(v)}</Badge> },
            {
              key: 'image_url',
              label: 'Image',
              render: (v) => (v ? '✓' : '—'),
            },
          ]}
        />
        <JsonDrawer
          opened={selected !== null}
          onClose={() => setSelected(null)}
          row={selected}
          title={selected?.name}
        />
      </Stack>
    </Container>
  );
}
