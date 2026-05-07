// `/orgs/{org_id}/projects/{project_id}` — project dashboard.
// First end-to-end exercise of the WorkspaceShell + the typed
// `getProject` endpoint. The dashboard body is a placeholder
// today; real summary widgets land in 7.6 / 7.8 alongside the
// rail's resource views.

import { useEffect } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { Code, Loader, Stack, Text, Title } from '@mantine/core';

import { apiClient } from '../api/client';
import { notifyApiError } from '../api/errors';
import { RequireAuth } from '../auth/RequireAuth';
import { WorkspaceShell } from '../components/WorkspaceShell';

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id')({
  component: () => (
    <RequireAuth>
      <ProjectDashboard />
    </RequireAuth>
  ),
});

function ProjectDashboard() {
  const { org_id, project_id } = Route.useParams();
  const projectQuery = useQuery({
    queryKey: ['project', org_id, project_id],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/orgs/{org_id}/projects/{project_id}', {
        params: { path: { org_id, project_id } },
      });
      if (error) throw error;
      return data;
    },
  });

  useEffect(() => {
    if (projectQuery.error) notifyApiError(projectQuery.error);
  }, [projectQuery.error]);

  return (
    <WorkspaceShell orgId={org_id} projectId={project_id} projectName={projectQuery.data?.name}>
      {projectQuery.isLoading ? (
        <Loader size="sm" />
      ) : projectQuery.data ? (
        <Stack gap="md">
          <Title order={2}>{projectQuery.data.name}</Title>
          <Text size="sm" c="dimmed">
            Phase 7.5 placeholder workspace. Real resource views land in 7.6 (read- only tables for
            advertisers, campaigns, flights, ads, creatives, sites, zones, taxonomy, templates, ad
            library). Decision tester lands in 7.13; reporting + event-flow inspector in 7.8.
          </Text>
          <Stack gap="xs">
            <Text fw={600}>Project metadata</Text>
            <Code block>{JSON.stringify(projectQuery.data, null, 2)}</Code>
          </Stack>
        </Stack>
      ) : (
        <Text c="red">Project not loaded.</Text>
      )}
    </WorkspaceShell>
  );
}
