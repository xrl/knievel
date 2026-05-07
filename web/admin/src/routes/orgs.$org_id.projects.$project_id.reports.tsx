// `/orgs/{org_id}/projects/{project_id}/reports` —
// Rollup overview. Phase 7.8.
//
// Placeholder body until knievel exposes a public read
// endpoint for the rollups computed by the leader-elected
// rollup task (Phase 3.24). The shell + time-bucket
// controls land here so the IA + URL surface is real; the
// chart fetches plug in once the API is there.
//
// Per UI.md "Information architecture" + REPORTING.md.

import { createFileRoute } from '@tanstack/react-router';
import { Alert, Anchor, Code, SegmentedControl, Stack, Text, Title } from '@mantine/core';
import { useState } from 'react';

import { RequireAuth } from '../auth/RequireAuth';
import { WorkspaceShell } from '../components/WorkspaceShell';

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/reports')({
  component: () => (
    <RequireAuth>
      <Reports />
    </RequireAuth>
  ),
});

function Reports() {
  const { org_id, project_id } = Route.useParams();
  const [bucket, setBucket] = useState<'hour' | 'day' | 'week'>('day');

  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      <Stack gap="lg">
        <Title order={2}>Reports</Title>
        <Stack gap="xs">
          <Text size="sm" fw={500}>
            Time bucket
          </Text>
          <SegmentedControl
            value={bucket}
            onChange={(v) => setBucket(v as typeof bucket)}
            data={[
              { value: 'hour', label: 'Hour' },
              { value: 'day', label: 'Day' },
              { value: 'week', label: 'Week' },
            ]}
          />
        </Stack>

        <Alert color="blue" variant="light" title="Awaiting backing endpoint">
          <Stack gap="xs">
            <Text size="sm">
              The rollup loop runs leader-side (Phase 3.24) but the project-scoped list endpoint
              that surfaces it isn't built yet — see <Code>REPORTING.md</Code> for the planned shape
              and <Code>PHASES.md</Code> for the open task. Once the endpoint lands, this view
              fetches with the time-bucket selector wired above.
            </Text>
            <Text size="sm">
              Need to debug a specific request now? Use the{' '}
              <Anchor href={`/orgs/${org_id}/projects/${project_id}/reports/test`}>
                decision tester
              </Anchor>{' '}
              — it covers the most common "why isn't this serving?" workflow without needing rollup
              data.
            </Text>
          </Stack>
        </Alert>
      </Stack>
    </WorkspaceShell>
  );
}
