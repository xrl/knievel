// Unified error → notification helper. Maps the API's error
// envelope + status code into the per-status UX from
// `UI.md` "Error handling / State machine per failure mode".
//
// v0 surfaces every non-field error as a Mantine
// notification with the X-Request-Id from the response so
// support can correlate with server logs. Inline panels for
// 403 and field-level mapping for 400/422 are folded into the
// 7.5+ views once they have real forms / detail shells; this
// helper is the single funnel that everything goes through
// until then.

import { notifications } from '@mantine/notifications';

import { getLastRequestId } from './client';

/** Shape of the API's error envelope. Mirrors the Rust-side
 *  `ErrorEnvelope` in `src/handlers.rs`. */
export interface ApiErrorEnvelope {
  error: {
    code: string;
    message?: string;
    details?: unknown;
  };
}

interface NotifyOpts {
  /** Override the title (default: derived from status). */
  title?: string;
  /** Override the body (default: error envelope's message). */
  body?: string;
  /** When set, treat as a network error rather than a
   *  status-keyed API error (no request_id available). */
  network?: boolean;
}

/** Show a notification for an API error. Reads the most
 *  recent X-Request-Id from the fetch wrapper (the request
 *  that produced the error) and appends it to the body. */
export function notifyApiError(err: unknown, opts: NotifyOpts = {}): void {
  const requestId = getLastRequestId();
  const status = extractStatus(err);
  const envelope = extractEnvelope(err);

  const title = opts.title ?? defaultTitleForStatus(status, opts.network);
  const fallbackMsg = envelope?.error.message ?? defaultBodyForStatus(status, opts.network);
  const body = opts.body ?? fallbackMsg;
  const withRequestId = requestId ? `${body}\nRequest ID: ${requestId}` : body;

  notifications.show({
    title,
    message: withRequestId,
    color: status && status >= 500 ? 'red' : status === 0 || opts.network ? 'orange' : 'red',
    autoClose: status && status >= 500 ? false : 5000,
    withCloseButton: true,
  });
}

function extractStatus(err: unknown): number | undefined {
  if (typeof err === 'object' && err !== null && 'status' in err) {
    const s = (err as { status?: unknown }).status;
    if (typeof s === 'number') return s;
  }
  return undefined;
}

function extractEnvelope(err: unknown): ApiErrorEnvelope | null {
  if (typeof err !== 'object' || err === null) return null;
  if (
    'error' in err &&
    typeof (err as { error?: unknown }).error === 'object' &&
    (err as { error?: unknown }).error !== null &&
    'code' in (err as { error: { code?: unknown } }).error
  ) {
    return err as ApiErrorEnvelope;
  }
  return null;
}

function defaultTitleForStatus(status: number | undefined, network?: boolean): string {
  if (network) return 'Network error';
  if (status === undefined) return 'Request failed';
  if (status === 400 || status === 422) return 'Invalid request';
  if (status === 401) return 'Sign-in required';
  if (status === 403) return 'Forbidden';
  if (status === 404) return 'Not found';
  if (status === 409) return 'Conflict';
  if (status === 429) return 'Too many requests';
  if (status >= 500) return 'Knievel returned an error';
  return `Request failed (${status})`;
}

function defaultBodyForStatus(status: number | undefined, network?: boolean): string {
  if (network) return "Couldn't reach knievel — check your connection.";
  if (status === undefined) return 'Try again, or contact support if the problem persists.';
  if (status === 401) return 'Your session has expired. Sign in again to continue.';
  if (status === 403) return "You don't have access to this resource.";
  if (status === 429) return 'Slow down — too many requests in a short window.';
  if (status >= 500) return 'The server returned an error. Try again or contact support.';
  return 'Request failed. Try again, or contact support if the problem persists.';
}
