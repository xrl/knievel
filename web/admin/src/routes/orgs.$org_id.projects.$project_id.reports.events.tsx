// `/orgs/{org_id}/projects/{project_id}/reports/events` —
// Events tail. Phase 7.8.
//
// Events_raw is internal to knievel; the public auth-fronted
// feed isn't built yet (the existing /e/i and /e/c endpoints
// are public, HMAC-signed click/impression trackers — they're
// what *generates* events, not how operators read them).
// When a project-scoped events list endpoint lands, this
// view replaces the placeholder with a poll-based tail
// (refetch every 5s) per UI.md "Information architecture /
// Reports."

import { createFileRoute } from '@tanstack/react-router';
import { Alert, Code, Stack, Text, Title } from '@mantine/core';

import { RequireAuth } from '../auth/RequireAuth';
import { WorkspaceShell } from '../components/WorkspaceShell';

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/reports/events')({
  component: () => (
    <RequireAuth>
      <EventsTail />
    </RequireAuth>
  ),
});

function EventsTail() {
  const { org_id, project_id } = Route.useParams();
  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      <Stack gap="md">
        <Title order={2}>Events</Title>
        <Alert color="blue" variant="light" title="Awaiting backing endpoint">
          <Text size="sm">
            Events flow into knievel's <Code>events_raw</Code> table via the HMAC-signed{' '}
            <Code>/e/i</Code> and <Code>/e/c</Code> trackers (see <Code>API.md</Code> § 4). A
            project-scoped feed for operators isn't exposed publicly yet; once it lands, this view
            becomes a poll- based tail of the last few hundred events.
          </Text>
        </Alert>
      </Stack>
    </WorkspaceShell>
  );
}
