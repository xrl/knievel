// Paste-a-token login form. Shown when:
//
// - OIDC is disabled in runtime config (empty issuer), OR
// - OIDC is enabled but `require_oidc: false` and the user
//   chose the fallback link from the OIDC login screen.
//
// Operators paste a `kvl_*` opaque token minted via
// `POST /v1/orgs/{org_id}/tokens`; we validate it against
// `GET /v1/whoami` and stash it in sessionStorage on success.
// Fully cleared on tab close — the storage helpers in
// `session.ts` use `sessionStorage`, not localStorage.

import { useState } from 'react';
import { Alert, Button, Container, PasswordInput, Stack, Text, Title } from '@mantine/core';

import { apiClient } from '../api/client';
import { setPasteToken } from './session';

interface Props {
  onSuccess: () => void;
}

export function PasteTokenLogin({ onSuccess }: Props) {
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!token.trim()) return;
    setBusy(true);
    setErr(null);
    setPasteToken(token.trim());
    try {
      const result = await apiClient.GET('/v1/whoami');
      const status = result.response.status;
      if (result.error || !result.data) {
        if (status === 401) {
          setErr('Token rejected. Check that it was copied in full and not revoked.');
        } else {
          setErr(`Unexpected response (${status}). Try again.`);
        }
        return;
      }
      onSuccess();
    } catch (e) {
      setErr(`Network error: ${(e as Error).message}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Container size="xs" py="xl">
      <form onSubmit={submit}>
        <Stack gap="md">
          <Title order={2}>Sign in</Title>
          <Text size="sm" c="dimmed">
            Paste an opaque token (<code>kvl_*</code>) minted via the org tokens API. The token is
            validated against <code>/v1/whoami</code> and kept in this tab's session storage — it
            disappears when the tab closes.
          </Text>
          <PasswordInput
            label="Token"
            placeholder="kvl_prod_org_…"
            value={token}
            onChange={(e) => setToken(e.currentTarget.value)}
            autoFocus
            data-testid="paste-token-input"
          />
          {err && (
            <Alert color="red" variant="light">
              {err}
            </Alert>
          )}
          <Button type="submit" loading={busy} disabled={!token.trim()}>
            Sign in
          </Button>
        </Stack>
      </form>
    </Container>
  );
}
