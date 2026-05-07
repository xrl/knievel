// Unit tests for `notifyApiError`. Mocks the Mantine
// notifications API so we can assert the right title / body /
// color is dispatched per status. The full visual UX (drawers,
// inline panels) is out of scope here — the helper's contract
// is "every API failure becomes a Mantine notification with
// the request_id appended."

import { describe, expect, it, vi } from 'vitest';

vi.mock('@mantine/notifications', () => ({
  notifications: {
    show: vi.fn(),
  },
}));

import { notifications } from '@mantine/notifications';

import { notifyApiError } from './errors';

describe('notifyApiError', () => {
  it('handles 401 with the sign-in-required title', () => {
    notifyApiError({ status: 401, error: { code: 'invalid_token' } });
    const last = (notifications.show as ReturnType<typeof vi.fn>).mock.calls.at(-1)?.[0];
    expect(last.title).toBe('Sign-in required');
  });

  it('handles 403 with the forbidden title', () => {
    notifyApiError({ status: 403, error: { code: 'role_insufficient' } });
    const last = (notifications.show as ReturnType<typeof vi.fn>).mock.calls.at(-1)?.[0];
    expect(last.title).toBe('Forbidden');
  });

  it('handles 5xx with red color and no auto-close', () => {
    notifyApiError({ status: 500, error: { code: 'internal' } });
    const last = (notifications.show as ReturnType<typeof vi.fn>).mock.calls.at(-1)?.[0];
    expect(last.color).toBe('red');
    expect(last.autoClose).toBe(false);
  });

  it('treats network errors with an orange tone', () => {
    notifyApiError(new Error('ECONNREFUSED'), { network: true });
    const last = (notifications.show as ReturnType<typeof vi.fn>).mock.calls.at(-1)?.[0];
    expect(last.title).toBe('Network error');
    expect(last.color).toBe('orange');
  });

  it('uses the envelope message when present', () => {
    notifyApiError({
      status: 409,
      error: { code: 'external_id_conflict', message: 'externalId already taken' },
    });
    const last = (notifications.show as ReturnType<typeof vi.fn>).mock.calls.at(-1)?.[0];
    expect(last.message).toContain('externalId already taken');
  });
});
