// `/orgs/{org_id}/projects/{project_id}/advertisers/new` —
// Create-advertiser form. Phase 7.7.
//
// The first end-to-end example of the editing surface: zod
// schema, react-hook-form, idempotency-key handling,
// optimistic invalidation. Other resource forms follow the
// same shape; the patterns lift to a shared
// `<ResourceCreateForm>` once they stabilize across 3+
// resources.

import { useState } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useForm } from 'react-hook-form';
import { zodResolver } from '@hookform/resolvers/zod';
import { z } from 'zod';
import { Alert, Button, Container, Group, Stack, TextInput, Title } from '@mantine/core';

import { apiClient } from '../api/client';
import { notifyApiError } from '../api/errors';
import { RequireAuth } from '../auth/RequireAuth';
import { WorkspaceShell } from '../components/WorkspaceShell';
import { newIdempotencyKey } from '../lib/idempotency';

const FormSchema = z.object({
  name: z.string().min(1, 'Name is required').max(200),
  external_id: z.string().max(200).optional(),
});
type FormValues = z.infer<typeof FormSchema>;

export const Route = createFileRoute('/orgs/$org_id/projects/$project_id/advertisers/new')({
  component: () => (
    <RequireAuth>
      <NewAdvertiser />
    </RequireAuth>
  ),
});

function NewAdvertiser() {
  const { org_id, project_id } = Route.useParams();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const {
    register,
    handleSubmit,
    formState: { errors, isSubmitting },
  } = useForm<FormValues>({
    resolver: zodResolver(FormSchema),
  });

  const [submitError, setSubmitError] = useState<string | null>(null);

  const create = useMutation({
    mutationFn: async (values: FormValues) => {
      // Idempotency-Key isn't in the createAdvertiser typed
      // header set today (server doesn't honor it on
      // advertisers — Phase 6.1 closes the POST idempotency
      // parity gap). Send it anyway via the untyped
      // `headers` option so when 6.1 lands the SPA gets the
      // replay window without a code change.
      const idempotencyKey = newIdempotencyKey();
      const { data, error } = await apiClient.POST('/v1/projects/{project_id}/advertisers', {
        params: { path: { project_id } },
        body: {
          name: values.name,
          external_id: values.external_id || undefined,
        },
        headers: { 'Idempotency-Key': idempotencyKey },
      });
      if (error) throw error;
      return data;
    },
    onSuccess: async (advertiser) => {
      // Invalidate the list query so the new row shows up
      // immediately on return navigation.
      await queryClient.invalidateQueries({ queryKey: ['advertisers', project_id] });
      navigate({
        to: `/orgs/${org_id}/projects/${project_id}/advertisers`,
        replace: true,
      });
      // Optional: could open the JsonDrawer pre-focused on
      // the new row. Defer that polish.
      void advertiser;
    },
    onError: (err) => {
      const status =
        typeof err === 'object' && err !== null && 'status' in err
          ? (err as { status?: unknown }).status
          : undefined;
      if (status === 409) {
        setSubmitError('Something with that external_id already exists in this project.');
      } else {
        setSubmitError('Create failed. See the toast for details.');
      }
      notifyApiError(err);
    },
  });

  function onSubmit(values: FormValues) {
    setSubmitError(null);
    create.mutate(values);
  }

  return (
    <WorkspaceShell orgId={org_id} projectId={project_id}>
      <Container size="sm" py="md">
        <form onSubmit={handleSubmit(onSubmit)}>
          <Stack gap="md">
            <Title order={2}>New advertiser</Title>
            <TextInput
              label="Name"
              placeholder="e.g. Acme Corp"
              required
              autoFocus
              error={errors.name?.message}
              {...register('name')}
            />
            <TextInput
              label="External ID"
              description="Caller-assigned. Unique within the project. Optional."
              placeholder="e.g. acme-corp"
              error={errors.external_id?.message}
              {...register('external_id')}
            />
            {submitError && (
              <Alert color="red" variant="light">
                {submitError}
              </Alert>
            )}
            <Group justify="flex-end">
              <Button
                variant="subtle"
                onClick={() =>
                  navigate({
                    to: `/orgs/${org_id}/projects/${project_id}/advertisers`,
                  })
                }
                type="button"
              >
                Cancel
              </Button>
              <Button type="submit" loading={isSubmitting || create.isPending}>
                Create
              </Button>
            </Group>
          </Stack>
        </form>
      </Container>
    </WorkspaceShell>
  );
}
