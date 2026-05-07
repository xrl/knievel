// Creative image upload. Phase 7.7.
//
// Drag-and-drop picker over `POST /v1/projects/{p}/creatives/{id}/image`
// (`src/image_upload.rs`). Client-side validation mirrors
// `images.upload.{max_bytes,allowed_mime_types}` from
// `config.example.yaml`; server still enforces — the
// client check is just a fast-fail UX.
//
// Knievel's defaults: 40 MiB max; jpeg, png, gif, webp,
// avif. If a deployment widens these via config, the server
// accepts the wider set and the client's pre-check is just
// conservative (rejects something the server would accept).
// Acceptable trade-off for v0; can be widened by surfacing
// the limits via /admin/config.json later if it bites.

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Dropzone, type FileWithPath, MIME_TYPES } from '@mantine/dropzone';
import { Group, Stack, Text } from '@mantine/core';

import { getCurrentBearer } from '../auth/session';
import { notifyApiError } from '../api/errors';

const MAX_BYTES = 40 * 1024 * 1024; // 40 MiB
const ALLOWED_MIME = [
  MIME_TYPES.jpeg,
  MIME_TYPES.png,
  MIME_TYPES.gif,
  MIME_TYPES.webp,
  'image/avif',
];

interface Props {
  projectId: string;
  creativeId: number;
}

export function CreativeImageUpload({ projectId, creativeId }: Props) {
  const queryClient = useQueryClient();
  const [progress, setProgress] = useState<string | null>(null);

  const upload = useMutation({
    mutationFn: async (file: File) => {
      // openapi-fetch's typed multipart support is uneven
      // across schema variants; use plain fetch with the
      // shared Bearer accessor for clarity. The auth +
      // X-Request-Id flows still work — we just don't go
      // through the typed `apiClient`.
      const fd = new FormData();
      fd.append('file', file);
      const bearer = getCurrentBearer();
      const headers: Record<string, string> = {};
      if (bearer) headers.authorization = `Bearer ${bearer}`;
      const resp = await fetch(
        `/v1/projects/${encodeURIComponent(projectId)}/creatives/${creativeId}/image`,
        { method: 'POST', headers, body: fd },
      );
      if (!resp.ok) {
        let envelope: unknown;
        try {
          envelope = await resp.json();
        } catch {
          envelope = null;
        }
        throw Object.assign(new Error('upload failed'), {
          status: resp.status,
          ...(typeof envelope === 'object' && envelope !== null ? envelope : {}),
        });
      }
      return resp.json();
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['creatives', projectId] });
      setProgress(null);
    },
    onError: (err) => {
      setProgress(null);
      notifyApiError(err);
    },
  });

  function handleDrop(files: FileWithPath[]) {
    const file = files[0];
    if (!file) return;
    if (file.size > MAX_BYTES) {
      notifyApiError(
        { status: 0, error: { code: 'too_large', message: `${file.name} is over 40 MiB` } },
        { network: false, title: 'File too large' },
      );
      return;
    }
    setProgress(`Uploading ${file.name}…`);
    upload.mutate(file);
  }

  return (
    <Stack gap="xs">
      <Dropzone
        onDrop={handleDrop}
        onReject={() => {
          notifyApiError({
            status: 0,
            error: {
              code: 'rejected',
              message: 'File type not allowed. JPEG, PNG, GIF, WebP, AVIF only.',
            },
          });
        }}
        maxSize={MAX_BYTES}
        accept={ALLOWED_MIME}
        loading={upload.isPending}
        multiple={false}
        data-testid="creative-image-dropzone"
      >
        <Group justify="center" gap="md" mih={120}>
          <Stack gap={4} align="center">
            <Text size="sm">Drop an image here, or click to browse.</Text>
            <Text size="xs" c="dimmed">
              JPEG / PNG / GIF / WebP / AVIF · ≤ 40 MiB
            </Text>
          </Stack>
        </Group>
      </Dropzone>
      {progress && (
        <Text size="sm" c="dimmed">
          {progress}
        </Text>
      )}
    </Stack>
  );
}
