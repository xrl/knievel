// `/orgs/{org_id}/projects/{project_id}/advertisers` —
// read-only list of advertisers. Phase 7.6.

import { useEffect, useState } from 'react';
import { createFileRoute, Link } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { Badge, Button, Group } from '@mantine/core';

import { apiClient } from '../api/client';
import type { components } from '../api/generated';
import { notifyApiError } from '../api/errors';
import { RequireAuth } from '../auth/RequireAuth';
import { hasRoleAtLeast } from '../auth/roles';
import { useWhoami } from '../auth/whoamiQuery';
import { WorkspaceShell } from '../components/WorkspaceShell';
import { DataTable } from '../components/DataTable';
import { JsonDrawer } from '../components/JsonDrawer';

type Advertiser = components['schemas']['Advertiser'];

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/advertisers')({
  component: () => (
    <RequireAuth>
      <Advertisers />
    </RequireAuth>
  ),
});

function Advertisers() {
  const { org_id, project_id } = Route.useParams();
  const whoami = useWhoami();
  const canEdit = hasRoleAtLeast(whoami.data?.role, 'editor');
  const { data, isLoading, error } = useQuery({
    queryKey: ['advertisers', project_id],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/projects/{project_id}/advertisers', {
        params: { path: { project_id } },
      });
      if (error) throw error;
      return data;
    },
  });
  const [selected, setSelected] = useState<Advertiser | null>(null);

  useEffect(() => {
    if (error) notifyApiError(error);
  }, [error]);

  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      {canEdit && (
        <Group justify="flex-end" mb="sm">
          <Button component={Link} to={`/orgs/${org_id}/projects/${project_id}/advertisers/new`}>
            New advertiser
          </Button>
        </Group>
      )}
      <DataTable
        title="Advertisers"
        description="Click a row to inspect the full record."
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
          {
            key: 'is_active',
            label: 'Status',
            render: (v) => <Badge color={v ? 'green' : 'gray'}>{v ? 'active' : 'inactive'}</Badge>,
          },
          { key: 'created_at', label: 'Created' },
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
