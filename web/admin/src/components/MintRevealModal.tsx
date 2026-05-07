// Token-mint show-once modal. Phase 7.7.
//
// Mint endpoints return server-only secrets exactly once
// (per AUTH.md "Opaque Tokens" — knievel stores the argon2id
// hash and the value is unrecoverable thereafter). Getting
// this UX wrong silently locks operators out, so the workflow
// is pinned:
//
// 1. Modal shows the plaintext value in monospace, with a
//    one-click copy button and a "**Save this now — it will
//    not be shown again**" callout.
// 2. Dismissal is gated behind an explicit "I've stored this"
//    checkbox. No X-close, no Esc-to-dismiss, no clickaway.
//    The Done button is disabled until the box is ticked.
// 3. On close, the consumer should clear the secret from
//    React state + the Query cache; this modal does NOT
//    keep its own copy of the value beyond the open lifetime.
//
// Same pattern reused for HMAC secret rotation and any
// future mint endpoint.

import { useState } from 'react';
import {
  Alert,
  Button,
  Checkbox,
  Code,
  CopyButton,
  Group,
  Modal,
  Stack,
  Text,
} from '@mantine/core';

interface Props {
  opened: boolean;
  onClose: () => void;
  /** The plaintext value returned by the mint endpoint.
   *  Caller is responsible for clearing it from React state
   *  and the Query cache after onClose. */
  secret: string | null;
  title?: string;
  description?: string;
}

export function MintRevealModal({
  opened,
  onClose,
  secret,
  title = 'Save this credential',
  description,
}: Props) {
  const [stored, setStored] = useState(false);

  function handleClose() {
    setStored(false);
    onClose();
  }

  return (
    <Modal
      opened={opened}
      // Block the obvious close paths — operators MUST tick
      // the "I've stored this" checkbox first. Empty
      // onClose makes the X / Esc / clickaway no-ops.
      onClose={() => {
        /* intentional no-op */
      }}
      withCloseButton={false}
      closeOnClickOutside={false}
      closeOnEscape={false}
      title={title}
      size="lg"
    >
      <Stack gap="md">
        {description && (
          <Text size="sm" c="dimmed">
            {description}
          </Text>
        )}
        <Alert color="red" variant="light" title="One-time reveal">
          This is the only time you'll see this value. Knievel stores only an argon2id hash; the
          plaintext is unrecoverable after this dialog closes. Copy it to your secret store now.
        </Alert>

        <Stack gap="xs">
          <Text size="sm" fw={500}>
            Value
          </Text>
          <Group gap="xs" wrap="nowrap">
            <Code style={{ flex: 1, wordBreak: 'break-all' }}>{secret ?? '—'}</Code>
            {secret && (
              <CopyButton value={secret} timeout={1500}>
                {({ copied, copy }) => (
                  <Button
                    onClick={copy}
                    variant={copied ? 'filled' : 'light'}
                    color={copied ? 'green' : 'blue'}
                    size="sm"
                  >
                    {copied ? 'Copied' : 'Copy'}
                  </Button>
                )}
              </CopyButton>
            )}
          </Group>
        </Stack>

        <Checkbox
          label="I've stored this value somewhere safe."
          checked={stored}
          onChange={(e) => setStored(e.currentTarget.checked)}
          data-testid="mint-stored-checkbox"
        />

        <Group justify="flex-end">
          <Button onClick={handleClose} disabled={!stored} data-testid="mint-done">
            Done
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
