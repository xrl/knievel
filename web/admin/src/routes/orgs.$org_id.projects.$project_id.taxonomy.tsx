// `/orgs/{org_id}/projects/{project_id}/taxonomy` —
// channels / priorities / ad-types as tabs. Phase 7.6.
// All three are project-scoped read-only inventory taxonomy
// per `API.md` § 3.x ("Read-only inventory taxonomy").

import { useEffect } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { Tabs } from '@mantine/core';

import { apiClient } from '../api/client';
import type { components } from '../api/generated';
import { notifyApiError } from '../api/errors';
import { RequireAuth } from '../auth/RequireAuth';
import { WorkspaceShell } from '../components/WorkspaceShell';
import { DataTable } from '../components/DataTable';

type Channel = components['schemas']['Channel'];
type Priority = components['schemas']['Priority'];
type AdType = components['schemas']['AdType'];

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/taxonomy')({
  component: () => (
    <RequireAuth>
      <Taxonomy />
    </RequireAuth>
  ),
});

function Taxonomy() {
  const { org_id, project_id } = Route.useParams();
  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      <Tabs defaultValue="channels">
        <Tabs.List>
          <Tabs.Tab value="channels">Channels</Tabs.Tab>
          <Tabs.Tab value="priorities">Priorities</Tabs.Tab>
          <Tabs.Tab value="ad-types">Ad types</Tabs.Tab>
        </Tabs.List>
        <Tabs.Panel value="channels" pt="md">
          <ChannelsTable projectId={project_id} />
        </Tabs.Panel>
        <Tabs.Panel value="priorities" pt="md">
          <PrioritiesTable projectId={project_id} />
        </Tabs.Panel>
        <Tabs.Panel value="ad-types" pt="md">
          <AdTypesTable projectId={project_id} />
        </Tabs.Panel>
      </Tabs>
    </WorkspaceShell>
  );
}

function ChannelsTable({ projectId }: { projectId: string }) {
  const { data, isLoading, error } = useQuery({
    queryKey: ['channels', projectId],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/projects/{project_id}/channels', {
        params: { path: { project_id: projectId } },
      });
      if (error) throw error;
      return data;
    },
  });
  useEffect(() => {
    if (error) notifyApiError(error);
  }, [error]);
  return (
    <DataTable<Channel>
      loading={isLoading}
      error={error}
      items={data?.items ?? []}
      rowKey={(r) => r.id}
      columns={[
        { key: 'name', label: 'Name' },
        { key: 'id', label: 'ID', render: (v) => <code>{String(v)}</code> },
      ]}
    />
  );
}

function PrioritiesTable({ projectId }: { projectId: string }) {
  const { data, isLoading, error } = useQuery({
    queryKey: ['priorities', projectId],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/projects/{project_id}/priorities', {
        params: { path: { project_id: projectId } },
      });
      if (error) throw error;
      return data;
    },
  });
  useEffect(() => {
    if (error) notifyApiError(error);
  }, [error]);
  return (
    <DataTable<Priority>
      loading={isLoading}
      error={error}
      items={data?.items ?? []}
      rowKey={(r) => r.id}
      columns={[
        { key: 'name', label: 'Name' },
        { key: 'tier', label: 'Tier' },
        { key: 'id', label: 'ID', render: (v) => <code>{String(v)}</code> },
      ]}
    />
  );
}

function AdTypesTable({ projectId }: { projectId: string }) {
  const { data, isLoading, error } = useQuery({
    queryKey: ['ad-types', projectId],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/projects/{project_id}/ad-types', {
        params: { path: { project_id: projectId } },
      });
      if (error) throw error;
      return data;
    },
  });
  useEffect(() => {
    if (error) notifyApiError(error);
  }, [error]);
  return (
    <DataTable<AdType>
      loading={isLoading}
      error={error}
      items={data?.items ?? []}
      rowKey={(r) => r.id}
      columns={[
        { key: 'name', label: 'Name' },
        { key: 'width', label: 'Width' },
        { key: 'height', label: 'Height' },
        { key: 'id', label: 'ID', render: (v) => <code>{String(v)}</code> },
      ]}
    />
  );
}
