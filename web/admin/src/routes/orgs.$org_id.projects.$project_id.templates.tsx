// `/orgs/{org_id}/projects/{project_id}/templates` —
// CreativeTemplates list. Phase 7.6.

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

type Template = components['schemas']['CreativeTemplate'];

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/templates')({
  component: () => (
    <RequireAuth>
      <Templates />
    </RequireAuth>
  ),
});

function Templates() {
  const { org_id, project_id } = Route.useParams();
  const { data, isLoading, error } = useQuery({
    queryKey: ['templates', project_id],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/projects/{project_id}/creative-templates', {
        params: { path: { project_id } },
      });
      if (error) throw error;
      return data;
    },
  });
  const [selected, setSelected] = useState<Template | null>(null);
  useEffect(() => {
    if (error) notifyApiError(error);
  }, [error]);

  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      <DataTable
        title="Creative templates"
        description="JSON-Schema documents that gate per-creative validation. Click a row for the full schema."
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
            key: 'template_engine',
            label: 'Engine',
            render: (v) => (v ? String(v) : '—'),
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
