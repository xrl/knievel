// `/orgs/{org_id}/projects/{project_id}/creatives` — Phase 7.6.

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

type Creative = components['schemas']['Creative'];

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/creatives')({
  component: () => (
    <RequireAuth>
      <Creatives />
    </RequireAuth>
  ),
});

function Creatives() {
  const { org_id, project_id } = Route.useParams();
  const { data, isLoading, error } = useQuery({
    queryKey: ['creatives', project_id],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/projects/{project_id}/creatives', {
        params: { path: { project_id } },
      });
      if (error) throw error;
      return data;
    },
  });
  const [selected, setSelected] = useState<Creative | null>(null);
  useEffect(() => {
    if (error) notifyApiError(error);
  }, [error]);

  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      <DataTable
        title="Creatives"
        loading={isLoading}
        error={error}
        items={data?.items ?? []}
        rowKey={(r) => r.id}
        onRowClick={(r) => setSelected(r)}
        columns={[
          { key: 'name', label: 'Name', render: (v) => (v ? String(v) : '—') },
          { key: 'id', label: 'ID', render: (v) => <code>{String(v)}</code> },
          { key: 'kind', label: 'Kind', render: (v) => <Badge>{String(v)}</Badge> },
          {
            key: 'advertiser_id',
            label: 'Advertiser',
            render: (v) => <code>{String(v)}</code>,
          },
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
        title={selected?.name ?? `Creative ${selected?.id}`}
      />
    </WorkspaceShell>
  );
}
