// `/` — entry redirect. Every signed-in user has exactly one
// org (per the v0 principal model); we send them straight to
// `/orgs/{org_id}` so the breadcrumb is grounded from the
// first navigation. Auth guard runs first; if no credential
// is present, RequireAuth bounces to /oidc/login or /login.

import { useEffect } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { Center, Loader } from '@mantine/core';

import { useWhoami } from '../auth/whoamiQuery';
import { RequireAuth } from '../auth/RequireAuth';

export const Route = createFileRoute('/')({
  component: () => (
    <RequireAuth>
      <HomeRedirect />
    </RequireAuth>
  ),
});

function HomeRedirect() {
  const { data } = useWhoami();
  const navigate = useNavigate();
  useEffect(() => {
    if (data) {
      navigate({ to: `/orgs/${data.org_id}`, replace: true });
    }
  }, [data, navigate]);

  return (
    <Center mih={200}>
      <Loader />
    </Center>
  );
}
