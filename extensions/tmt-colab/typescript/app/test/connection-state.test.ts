import { describe, expect, it } from 'vite-plus/test';
import { REMOTE_REFUSAL_CODES, SessionEndedError, SessionEvictedError } from '../src/ask-remote.js';
import { SYNC_ERROR_CODES } from '../src/connection.js';
import { classify, CONNECTION_CODES, SYNC_DISCONNECTED } from '../src/connection-state.js';
import { ReopenFailedError } from '../src/registration.js';

const of = (code: string) => classify(new Error(code));

describe('connection classifier', () => {
  // Behavior-equality table: each row is what live.ts, readmit.ts and
  // terminal-failure.ts decided separately before one table owned them.
  it.each([
    [SYNC_DISCONNECTED, 'resync', 'generic'],
    ['RESYNC_REQUIRED', 'resync', 'generic'],
    ['Fresh membership catchup required', 'resync', 'generic'],
    ['Remote session ended', 'session', 'generic'],
    ['STALE_EPOCH', 'reopen', 'access'],
    ['DENIED', 'reopen', 'access'],
    ['EXPIRED', 'reopen', 'access'],
    ['Access ended', 'none', 'access'],
    ['REMOTE_SCOPE_DENIED', 'none', 'access'],
    // Remote's legacy close is never revoke proof (#2619): one silent re-admission first.
    ['REMOTE_CLOSED', 'session', 'access'],
    ['REMOTE_GRANT_CHANGED', 'session', 'access'],
    ['REMOTE_SERVE_STOPPED', 'session', 'generic'],
    ['REMOTE_DEVICE_REVOKED', 'none', 'access'],
    ['REMOTE_DEVICE_NOT_FOUND', 'none', 'access'],
    ['Page unavailable', 'none', 'gone'],
    ['REMOTE_SESSION_ENDED', 'none', 'session'],
    ['REMOTE_SEQUENCE_UNAVAILABLE', 'none', 'session'],
    ['CAPACITY', 'none', 'large'],
    ['Sync message capacity', 'none', 'large'],
    ['REMOTE_INPUT_TOO_LARGE', 'none', 'large'],
    ['INVALID', 'none', 'generic'],
    ['GAP', 'none', 'generic'],
    ['CONFLICT', 'none', 'generic'],
    ['REMOTE_INPUT_INVALID', 'none', 'generic'],
    ['REMOTE_RATE_LIMITED', 'none', 'generic'],
    ['REMOTE_REPLAY', 'none', 'generic'],
    ['REMOTE_INTENT_CONFLICT', 'none', 'generic'],
    ['REMOTE_STATE_UNAVAILABLE', 'none', 'generic'],
    ['REMOTE_CORE_UNAVAILABLE', 'none', 'generic'],
    ['SURPRISE_CODE', 'none', 'generic'],
  ])('%s recovers by %s and ends as %s', (code, recovery, cause) => {
    expect(of(code)).toEqual({ recovery, cause });
  });

  it('recovers a typed session end by replacing the session, whatever its code', () => {
    expect(classify(new SessionEndedError('REMOTE_SESSION_ENDED'))).toEqual({
      recovery: 'session',
      cause: 'session',
    });
    expect(classify(new SessionEndedError('REMOTE_SEQUENCE_UNAVAILABLE')).recovery).toBe('session');
  });

  // Remote's typed fresh-open refusal (R2): a transient reason keeps trying, a named one ends.
  it.each([
    ['unreachable', 'retry', 'generic'],
    ['unconfirmed', 'retry', 'generic'],
    ['transient', 'retry', 'generic'],
    ['unpaired', 'none', 'unpaired'],
    ['mismatch', 'none', 'unpaired'],
    ['revoked', 'none', 'access'],
    ['expired', 'none', 'access'],
    // The cap notice is C1c (#2625): until then it stays a plain ended page.
    ['capacity', 'none', 'generic'],
  ] as const)(
    'classifies a fresh open refused as %s: %s, ends as %s',
    (reason, recovery, cause) => {
      expect(classify(new ReopenFailedError(reason))).toEqual({ recovery, cause });
    },
  );

  it('never recovers an eviction: the session limit has its own card', () => {
    expect(classify(new SessionEvictedError(8)).recovery).toBe('none');
  });

  // A code added to the wire or Remote lists must get a decision here, instead
  // of shipping to readers as a raw headline or an accidental retry.
  it('has a decision for every sync frame code and Remote refusal code', () => {
    for (const code of [...SYNC_ERROR_CODES, ...REMOTE_REFUSAL_CODES]) {
      if (code === 'REMOTE_SESSION_EVICTED') continue;
      expect(CONNECTION_CODES, code).toHaveProperty(code);
    }
  });
});
