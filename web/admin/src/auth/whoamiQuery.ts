// Shared `useQuery` for `/v1/whoami` — every authenticated
// route needs the principal (org_id, role, etc.) so we cache
// it once per session at a long staleTime. The sign-out flow
// invalidates it.

import { useQuery } from '@tanstack/react-query';

import { apiClient } from '../api/client';

export const WHOAMI_QUERY_KEY = ['whoami'] as const;

export function useWhoami() {
  return useQuery({
    queryKey: WHOAMI_QUERY_KEY,
    queryFn: async () => {
      const { data, error } = await apiClient.GET('/v1/whoami');
      if (error) throw error;
      return data;
    },
    staleTime: 5 * 60 * 1000, // 5 minutes — principal rarely changes mid-session
  });
}
