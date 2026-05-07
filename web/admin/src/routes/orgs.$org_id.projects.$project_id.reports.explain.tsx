// `/orgs/{org_id}/projects/{project_id}/reports/explain` —
// Decision explainer. Phase 7.8.
//
// The :explain workflow is already covered by 7.13's
// decision tester (which runs both `/decisions` and
// `:explain` from the same form). Rather than fork the form,
// `/reports/explain` redirects to the tester. If a
// dedicated explainer-only surface emerges as a real
// operator workflow, this can grow its own form.

import { useEffect } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { Center, Loader } from '@mantine/core';

import { RequireAuth } from '../auth/RequireAuth';

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/reports/explain')({
  component: () => (
    <RequireAuth>
      <RedirectToTester />
    </RequireAuth>
  ),
});

function RedirectToTester() {
  const { org_id, project_id } = Route.useParams();
  const navigate = useNavigate();
  useEffect(() => {
    navigate({
      to: `/orgs/${org_id}/projects/${project_id}/reports/test`,
      replace: true,
    });
  }, [org_id, project_id, navigate]);
  return (
    <Center mih={200}>
      <Loader size="sm" />
    </Center>
  );
}
