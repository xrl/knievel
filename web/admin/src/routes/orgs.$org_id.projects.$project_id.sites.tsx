// `/orgs/{org_id}/projects/{project_id}/sites` — Phase 7.6.

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

type Site = components['schemas']['Site'];

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/sites')({
  component: () => (
    <RequireAuth>
      <Sites />
    </RequireAuth>
  ),
});

function Sites() {
  const { org_id, project_id } = Route.useParams();
  const { data, isLoading, error } = useQuery({
    queryKey: ['sites', project_id],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/projects/{project_id}/sites', {
        params: { path: { project_id } },
      });
      if (error) throw error;
      return data;
    },
  });
  const [selected, setSelected] = useState<Site | null>(null);
  useEffect(() => {
    if (error) notifyApiError(error);
  }, [error]);

  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      <DataTable
        title="Sites"
        loading={isLoading}
        error={error}
        items={data?.items ?? []}
        rowKey={(r) => r.id}
        onRowClick={(r) => setSelected(r)}
        columns={[
          { key: 'name', label: 'Name' },
          { key: 'url', label: 'URL', render: (v) => <code>{String(v)}</code> },
          { key: 'id', label: 'ID', render: (v) => <code>{String(v)}</code> },
          {
            key: 'channel_id',
            label: 'Channel',
            render: (v) => (v ? <code>{String(v)}</code> : '—'),
          },
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
        title={selected?.name}
      />
    </WorkspaceShell>
  );
}
