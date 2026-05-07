// Idle-warning modal. Phase 7.9.
//
// `oidc-client-ts` emits `addAccessTokenExpiring` 60 s
// before the access token expires (configurable via
// `accessTokenExpiringNotificationTimeInSeconds`). We
// surface a modal with a "Stay signed in" button that
// triggers `signinSilent()` to refresh the token without a
// full redirect. If the user takes no action, the token
// expires and the next request hits the 401 silent-refresh
// path from 7.4 — they'll either auto-refresh or be sent
// to login. The modal here is just the explicit
// "you're about to log out" UX.
//
// Mounted once at the app root inside <AuthProvider>; the
// hook is a no-op when OIDC isn't configured (paste-token
// mode has no expiry to warn about).

import { useEffect, useState } from 'react';
import { Button, Group, Modal, Stack, Text } from '@mantine/core';
import { useAuth } from 'react-oidc-context';

import { getRuntimeConfig, oidcEnabled } from './runtimeConfig';

export function IdleWarning() {
  const cfg = getRuntimeConfig();
  if (!cfg || !oidcEnabled(cfg)) return null;
  return <ActiveWarning />;
}

function ActiveWarning() {
  const auth = useAuth();
  const [warning, setWarning] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    if (!auth.isAuthenticated) return;
    // Wire the manager's events. `addAccessTokenExpiring`
    // returns an unsubscribe function in oidc-client-ts 3+.
    const onExpiring = () => setWarning(true);
    const onLoaded = () => setWarning(false);
    const onUnloaded = () => setWarning(false);
    auth.events.addAccessTokenExpiring(onExpiring);
    auth.events.addUserLoaded(onLoaded);
    auth.events.addUserUnloaded(onUnloaded);
    return () => {
      auth.events.removeAccessTokenExpiring(onExpiring);
      auth.events.removeUserLoaded(onLoaded);
      auth.events.removeUserUnloaded(onUnloaded);
    };
  }, [auth]);

  async function stayIn() {
    setRefreshing(true);
    try {
      await auth.signinSilent();
      setWarning(false);
    } finally {
      setRefreshing(false);
    }
  }

  return (
    <Modal
      opened={warning}
      onClose={() => setWarning(false)}
      withCloseButton
      title="Session expiring"
      size="md"
    >
      <Stack gap="md">
        <Text size="sm">
          Your session is about to expire. Click "Stay signed in" to refresh without leaving this
          page. Otherwise you'll be redirected to sign in again on the next request.
        </Text>
        <Group justify="flex-end">
          <Button variant="subtle" onClick={() => setWarning(false)}>
            Dismiss
          </Button>
          <Button onClick={stayIn} loading={refreshing}>
            Stay signed in
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
