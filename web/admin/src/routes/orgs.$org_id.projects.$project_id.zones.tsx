// `/orgs/{org_id}/projects/{project_id}/zones` — Phase 7.6.

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

type Zone = components['schemas']['Zone'];

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/zones')({
  component: () => (
    <RequireAuth>
      <Zones />
    </RequireAuth>
  ),
});

function Zones() {
  const { org_id, project_id } = Route.useParams();
  const { data, isLoading, error } = useQuery({
    queryKey: ['zones', project_id],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/projects/{project_id}/zones', {
        params: { path: { project_id } },
      });
      if (error) throw error;
      return data;
    },
  });
  const [selected, setSelected] = useState<Zone | null>(null);
  useEffect(() => {
    if (error) notifyApiError(error);
  }, [error]);

  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      <DataTable
        title="Zones"
        loading={isLoading}
        error={error}
        items={data?.items ?? []}
        rowKey={(r) => r.id}
        onRowClick={(r) => setSelected(r)}
        columns={[
          { key: 'name', label: 'Name' },
          { key: 'id', label: 'ID', render: (v) => <code>{String(v)}</code> },
          {
            key: 'site_id',
            label: 'Site ID',
            render: (v) => <code>{String(v)}</code>,
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
