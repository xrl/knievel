// `/orgs/{org_id}/projects/{project_id}/reports/test` —
// Decision tester. Phase 7.13.
//
// Lets an operator construct a real `POST /v1/projects/{p}/decisions`
// from a typed form, fire it, and render the served ads
// alongside the `:explain` response showing per-flight
// reasons. The single most-valuable surface for "why isn't my
// campaign serving?" debugging.
//
// `force.*` overrides are NOT exposed in the form today —
// they're admin-only via `API.md` § 1's three-control gate
// (decisions.force_overrides_enabled + per-project
// allow_force_decision + caller's role), and the SPA shouldn't
// dangle them in front of editor/reader users. Forced calls
// remain available via the API directly when an admin really
// needs them.

import { useState } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { Alert, Button, Code, Group, NumberInput, Stack, TextInput, Title } from '@mantine/core';

import { apiClient } from '../api/client';
import { notifyApiError } from '../api/errors';
import { RequireAuth } from '../auth/RequireAuth';
import { WorkspaceShell } from '../components/WorkspaceShell';

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/reports/test')({
  component: () => (
    <RequireAuth>
      <DecisionTester />
    </RequireAuth>
  ),
});

interface FormState {
  placement_id: string;
  site_external_id: string;
  zone_ids: string; // comma-separated
  ad_type_ids: string; // comma-separated
  count: number;
  context_url: string;
}

function parseIdList(s: string): number[] {
  return s
    .split(',')
    .map((x) => x.trim())
    .filter((x) => x.length > 0)
    .map((x) => Number(x))
    .filter((n) => Number.isFinite(n));
}

function DecisionTester() {
  const { org_id, project_id } = Route.useParams();

  const [form, setForm] = useState<FormState>({
    placement_id: 'div-1',
    site_external_id: '',
    zone_ids: '',
    ad_type_ids: '',
    count: 1,
    context_url: '',
  });

  const [decisionResult, setDecisionResult] = useState<unknown>(null);
  const [explainResult, setExplainResult] = useState<unknown>(null);
  const [busy, setBusy] = useState<'decision' | 'explain' | null>(null);

  function buildBody() {
    return {
      placements: [
        {
          id: form.placement_id,
          site_external_id: form.site_external_id || undefined,
          zone_ids: parseIdList(form.zone_ids),
          ad_types: parseIdList(form.ad_type_ids),
          count: form.count,
        },
      ],
      context: form.context_url ? { url: form.context_url } : undefined,
    };
  }

  async function runDecision() {
    setBusy('decision');
    setDecisionResult(null);
    try {
      const { data, error } = await apiClient.POST('/v1/projects/{project_id}/decisions', {
        params: { path: { project_id } },
        body: buildBody(),
      });
      if (error) {
        notifyApiError(error);
        return;
      }
      setDecisionResult(data);
    } finally {
      setBusy(null);
    }
  }

  async function runExplain() {
    setBusy('explain');
    setExplainResult(null);
    try {
      const { data, error } = await apiClient.POST('/v1/projects/{project_id}/decisions:explain', {
        params: { path: { project_id } },
        body: buildBody(),
      });
      if (error) {
        notifyApiError(error);
        return;
      }
      setExplainResult(data);
    } finally {
      setBusy(null);
    }
  }

  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      <Stack gap="lg">
        <Title order={2}>Decision tester</Title>
        <Alert color="blue" variant="light">
          Construct a real <code>POST /decisions</code> request, fire it, and inspect the served ads
          alongside the <code>:explain</code> reasons. Force overrides aren't exposed here — they
          require admin role and the per-project <code>allow_force_decision</code> flag (see{' '}
          <code>API.md</code> § 1).
        </Alert>

        <Stack gap="sm">
          <Title order={4}>Placement</Title>
          <Group grow>
            <TextInput
              label="Placement ID"
              description="Echoed back in the response under decisions[id]"
              value={form.placement_id}
              onChange={(e) => setForm({ ...form, placement_id: e.currentTarget.value })}
            />
            <TextInput
              label="Site (external_id)"
              description="Resolves the site for targeting"
              value={form.site_external_id}
              onChange={(e) => setForm({ ...form, site_external_id: e.currentTarget.value })}
            />
          </Group>
          <Group grow>
            <TextInput
              label="Zone IDs"
              description="Comma-separated numeric zone IDs"
              value={form.zone_ids}
              onChange={(e) => setForm({ ...form, zone_ids: e.currentTarget.value })}
            />
            <TextInput
              label="Ad type IDs"
              description="Comma-separated numeric ad-type IDs"
              value={form.ad_type_ids}
              onChange={(e) => setForm({ ...form, ad_type_ids: e.currentTarget.value })}
            />
          </Group>
          <Group grow>
            <NumberInput
              label="Count"
              description="Number of ads to return per placement"
              min={1}
              max={10}
              value={form.count}
              onChange={(v) => setForm({ ...form, count: typeof v === 'number' ? v : 1 })}
            />
            <TextInput
              label="Context URL"
              description="Optional — populates context.url for targeting"
              value={form.context_url}
              onChange={(e) => setForm({ ...form, context_url: e.currentTarget.value })}
            />
          </Group>
        </Stack>

        <Group>
          <Button onClick={runDecision} loading={busy === 'decision'} disabled={busy !== null}>
            Run decision
          </Button>
          <Button
            onClick={runExplain}
            variant="light"
            loading={busy === 'explain'}
            disabled={busy !== null}
          >
            Run :explain
          </Button>
        </Group>

        {decisionResult !== null && (
          <Stack gap="xs">
            <Title order={4}>Decision response</Title>
            <Code block>{JSON.stringify(decisionResult, null, 2)}</Code>
          </Stack>
        )}
        {explainResult !== null && (
          <Stack gap="xs">
            <Title order={4}>Explain response</Title>
            <Code block>{JSON.stringify(explainResult, null, 2)}</Code>
          </Stack>
        )}
      </Stack>
    </WorkspaceShell>
  );
}
