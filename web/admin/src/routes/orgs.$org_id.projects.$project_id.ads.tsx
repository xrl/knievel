// `/orgs/{org_id}/projects/{project_id}/ads` — Phase 7.6.

import { useEffect, useState } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { Badge } from '@mantine/core';

import { apiClient } from '../api/client';
import type { components } from '../api/generated';
import { notifyApiError } from '../api/errors';
import { RequireAuth } from '../auth/RequireAuth';
import { WorkspaceShell } from '../components/WorkspaceShell';
import { DataTable } from '../components/DataTable';
import { JsonDrawer } from '../components/JsonDrawer';

type Ad = components['schemas']['Ad'];

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/ads')({
  component: () => (
    <RequireAuth>
      <Ads />
    </RequireAuth>
  ),
});

function Ads() {
  const { org_id, project_id } = Route.useParams();
  const { data, isLoading, error } = useQuery({
    queryKey: ['ads', project_id],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/projects/{project_id}/ads', {
        params: { path: { project_id } },
      });
      if (error) throw error;
      return data;
    },
  });
  const [selected, setSelected] = useState<Ad | null>(null);
  useEffect(() => {
    if (error) notifyApiError(error);
  }, [error]);

  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      <DataTable
        title="Ads"
        loading={isLoading}
        error={error}
        items={data?.items ?? []}
        rowKey={(r) => r.id}
        onRowClick={(r) => setSelected(r)}
        columns={[
          { key: 'id', label: 'ID', render: (v) => <code>{String(v)}</code> },
          {
            key: 'flight_id',
            label: 'Flight ID',
            render: (v) => <code>{String(v)}</code>,
          },
          {
            key: 'creative_id',
            label: 'Creative ID',
            render: (v) => (v ? <code>{String(v)}</code> : '—'),
          },
          {
            key: 'ad_library_item_id',
            label: 'Library Item',
            render: (v) => (v ? <code>{String(v)}</code> : '—'),
          },
          { key: 'weight', label: 'Weight' },
          {
            key: 'is_active',
            label: 'Status',
            render: (v) => <Badge color={v ? 'green' : 'gray'}>{v ? 'active' : 'inactive'}</Badge>,
          },
        ]}
      />
      <JsonDrawer
        opened={selected !== null}
        onClose={() => setSelected(null)}
        row={selected}
        title={`Ad ${selected?.id}`}
      />
    </WorkspaceShell>
  );
}
