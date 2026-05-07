// `/orgs/{org_id}/projects/{project_id}/flights` — Phase 7.6.

import { useEffect, useState } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';

import { apiClient } from '../api/client';
import type { components } from '../api/generated';
import { notifyApiError } from '../api/errors';
import { RequireAuth } from '../auth/RequireAuth';
import { WorkspaceShell } from '../components/WorkspaceShell';
import { DataTable } from '../components/DataTable';
import { JsonDrawer } from '../components/JsonDrawer';

type Flight = components['schemas']['Flight'];

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/flights')({
  component: () => (
    <RequireAuth>
      <Flights />
    </RequireAuth>
  ),
});

function Flights() {
  const { org_id, project_id } = Route.useParams();
  const { data, isLoading, error } = useQuery({
    queryKey: ['flights', project_id],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/projects/{project_id}/flights', {
        params: { path: { project_id } },
      });
      if (error) throw error;
      return data;
    },
  });
  const [selected, setSelected] = useState<Flight | null>(null);
  useEffect(() => {
    if (error) notifyApiError(error);
  }, [error]);

  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      <DataTable
        title="Flights"
        loading={isLoading}
        error={error}
        items={data?.items ?? []}
        rowKey={(r) => r.id}
        onRowClick={(r) => setSelected(r)}
        columns={[
          { key: 'name', label: 'Name' },
          { key: 'id', label: 'ID', render: (v) => <code>{String(v)}</code> },
          {
            key: 'campaign_id',
            label: 'Campaign ID',
            render: (v) => <code>{String(v)}</code>,
          },
          {
            key: 'priority_id',
            label: 'Priority',
            render: (v) => <code>{String(v)}</code>,
          },
          { key: 'start_date', label: 'Start' },
          { key: 'end_date', label: 'End' },
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
