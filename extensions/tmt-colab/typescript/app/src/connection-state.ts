import { SessionEndedError } from './ask-remote.js';
import { AdmissionRefusedError, ReopenFailedError } from './registration.js';
import { SessionLimitError } from './session-limit-error.js';

/** Why a live page ended for good. Each cause owns one `strings.ts` sentence. */
export type FailureCause =
  | 'access'
  | 'gone'
  | 'session'
  | 'large'
  | 'unpaired'
  | 'capacity'
  | 'generic';

/**
 * What the page does about a failure before it gives up:
 * - `resync`: the tunnel dropped or the fold needs catchup; reconnect on the same session.
 * - `session`: the Remote session ended; open a replacement.
 * - `reopen`: the epoch advanced under a paired owner; bind the new epoch (#2557).
 * - `retry`: a fresh open was refused for a reason that may pass; keep trying.
 * - `none`: nothing recovers it.
 */
export type Recovery = 'resync' | 'session' | 'reopen' | 'retry' | 'none';

export interface Classification {
  readonly recovery: Recovery;
  /** The sentence the card shows if recovery is exhausted or `none`. */
  readonly cause: FailureCause;
}

/** The opaque message Connection gives a socket error or close. */
export const SYNC_DISCONNECTED = 'Sync disconnected';

const end = (cause: FailureCause): Classification => ({ recovery: 'none', cause });
const generic = end('generic');
const recover = (recovery: Recovery, cause: FailureCause = 'generic'): Classification => ({
  recovery,
  cause,
});

/**
 * Every raw token the live path can fail with, decided once. A token absent here is unknown:
 * it is not recovered, gets the generic sentence and stays visible as the reference.
 */
export const CONNECTION_CODES: Readonly<Record<string, Classification>> = {
  [SYNC_DISCONNECTED]: recover('resync'),
  RESYNC_REQUIRED: recover('resync'),
  'Fresh membership catchup required': recover('resync'),
  'Remote session ended': recover('session'),
  // None says the device lost access: reopening binds the new epoch, and a revoked
  // device is refused by that fresh open, which is terminal.
  STALE_EPOCH: recover('reopen', 'access'),
  DENIED: recover('reopen', 'access'),
  EXPIRED: recover('reopen', 'access'),
  'Access ended': end('access'),
  REMOTE_SCOPE_DENIED: end('access'),
  // Remote's close is never revoke proof (#2619): one silent re-admission first, and the card
  // only when the fresh open is refused.
  REMOTE_CLOSED: recover('session', 'access'),
  REMOTE_GRANT_CHANGED: recover('session', 'access'),
  REMOTE_SERVE_STOPPED: recover('session'),
  REMOTE_DEVICE_REVOKED: end('access'),
  REMOTE_DEVICE_NOT_FOUND: end('access'),
  'Page unavailable': end('gone'),
  REMOTE_SESSION_ENDED: end('session'),
  REMOTE_SEQUENCE_UNAVAILABLE: end('session'),
  CAPACITY: end('large'),
  'Sync message capacity': end('large'),
  REMOTE_INPUT_TOO_LARGE: end('large'),
  // Internal refusals that reach the card only when nothing recovered them.
  INVALID: generic,
  GAP: generic,
  CONFLICT: generic,
  REMOTE_INPUT_INVALID: generic,
  REMOTE_RATE_LIMITED: generic,
  REMOTE_REPLAY: generic,
  REMOTE_INTENT_CONFLICT: generic,
  REMOTE_STATE_UNAVAILABLE: generic,
  REMOTE_CORE_UNAVAILABLE: generic,
};

/** HTTP statuses with which the door's mount reports that the extension behind it is not serving. */
const UNREACHABLE_STATUSES: readonly number[] = [404, 502, 503, 504];

/** The decision for each reason Remote gives when a fresh open fails (R2). */
const REOPEN_REASONS: Readonly<Record<ReopenFailedError['reason'], Classification>> = {
  unreachable: recover('retry'),
  unconfirmed: recover('retry'),
  transient: recover('retry'),
  unpaired: end('unpaired'),
  mismatch: end('unpaired'),
  revoked: end('access'),
  expired: end('access'),
  // The verified limit refusal arrives as a `SessionLimitError` (registration.ts) and has its own
  // card; a capacity reason without one is unverifiable and ends as a plain failure.
  capacity: generic,
};

/** The single decision for a live-path failure; callers never read raw messages. */
export function classify(error: Error): Classification {
  if (error instanceof ReopenFailedError) return REOPEN_REASONS[error.reason];
  // Nothing recovers it inside the page: Remote does not retry a limit refusal, and a reload
  // after another session frees is the reader's one action.
  if (error instanceof SessionLimitError) return end('capacity');
  if (error instanceof AdmissionRefusedError) {
    // Unlike an old transport closing, a named refusal of the fresh admission ends access.
    if (error.code === 'REMOTE_CLOSED') return end('access');
    const named =
      error.code && Object.hasOwn(CONNECTION_CODES, error.code)
        ? CONNECTION_CODES[error.code]
        : undefined;
    // A named ending keeps its reason.
    if (named?.recovery === 'none' && named.cause !== 'generic') return named;
    // The mount answering for an extension that is not there (restarting or upgrading) says
    // nothing about the device, so it waits in the silent transport loop.
    if (!named && UNREACHABLE_STATUSES.includes(error.status)) return recover('retry');
    // A stale or unclassified HTTP admission uses the existing finite reopen owner.
    return recover('reopen');
  }
  const known = Object.hasOwn(CONNECTION_CODES, error.message)
    ? CONNECTION_CODES[error.message]!
    : generic;
  // A typed session end replaces the session whatever its code; an eviction is not in
  // the table, so it stays unrecovered and the page shows its own session-limit card.
  return error instanceof SessionEndedError ? { ...known, recovery: 'session' } : known;
}
