// `/orgs/{org_id}` — org dashboard. List of projects under
// the org with a click-through to the project workspace.

import { useEffect } from 'react';
import { createFileRoute, Link } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { Anchor, Badge, Container, Group, Loader, Stack, Table, Text, Title } from '@mantine/core';

import { apiClient } from '../api/client';
import { notifyApiError } from '../api/errors';
import { RequireAuth } from '../auth/RequireAuth';

export const Route = createFileRoute('/orgs/$org_id')({
  component: () => (
    <RequireAuth>
      <OrgDashboard />
    </RequireAuth>
  ),
});

function OrgDashboard() {
  const { org_id } = Route.useParams();

  const orgQuery = useQuery({
    queryKey: ['org', org_id],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/orgs/{org_id}', {
        params: { path: { org_id } },
      });
      if (error) throw error;
      return data;
    },
  });

  const projectsQuery = useQuery({
    queryKey: ['projects', org_id],
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/orgs/{org_id}/projects', {
        params: { path: { org_id } },
      });
      if (error) throw error;
      return data;
    },
  });

  useEffect(() => {
    if (orgQuery.error) notifyApiError(orgQuery.error);
  }, [orgQuery.error]);
  useEffect(() => {
    if (projectsQuery.error) notifyApiError(projectsQuery.error);
  }, [projectsQuery.error]);

  return (
    <Container size="lg" py="xl">
      <Stack gap="lg">
        <Group justify="space-between" align="flex-end">
          <Stack gap={4}>
            <Title order={1}>{orgQuery.data?.name ?? org_id}</Title>
            {orgQuery.data?.external_id && (
              <Text c="dimmed" size="sm">
                external_id: <code>{orgQuery.data.external_id}</code>
              </Text>
            )}
          </Stack>
          <Anchor component={Link} to="/oidc/logout" size="sm">
            Sign out
          </Anchor>
        </Group>

        <Stack gap="xs">
          <Title order={3}>Projects</Title>
          {projectsQuery.isLoading ? (
            <Loader size="sm" />
          ) : projectsQuery.data?.items?.length ? (
            <Table highlightOnHover striped withTableBorder>
              <Table.Thead>
                <Table.Tr>
                  <Table.Th>Name</Table.Th>
                  <Table.Th>ID</Table.Th>
                  <Table.Th>External ID</Table.Th>
                  <Table.Th>Status</Table.Th>
                </Table.Tr>
              </Table.Thead>
              <Table.Tbody>
                {projectsQuery.data.items.map((p) => (
                  <Table.Tr key={p.id}>
                    <Table.Td>
                      <Anchor component={Link} to={`/orgs/${org_id}/projects/${p.id}`}>
                        {p.name}
                      </Anchor>
                    </Table.Td>
                    <Table.Td>
                      <code>{p.id}</code>
                    </Table.Td>
                    <Table.Td>{p.external_id ? <code>{p.external_id}</code> : '—'}</Table.Td>
                    <Table.Td>
                      <Badge color={p.is_active ? 'green' : 'gray'}>
                        {p.is_active ? 'active' : 'inactive'}
                      </Badge>
                    </Table.Td>
                  </Table.Tr>
                ))}
              </Table.Tbody>
            </Table>
          ) : (
            <Text c="dimmed">No projects yet.</Text>
          )}
        </Stack>
      </Stack>
    </Container>
  );
}
