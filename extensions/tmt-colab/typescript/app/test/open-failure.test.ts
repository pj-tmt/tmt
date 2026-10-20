import { describe, expect, it } from 'vite-plus/test';
import { CONNECTION_CODES } from '../src/connection-state.js';
import { openFailure } from '../src/open-failure.js';
import { AdmissionRefusedError, ReopenFailedError } from '../src/registration.js';
import { SessionLimitError } from '../src/session-limit-error.js';
import { text } from '../src/strings.js';

describe('how a page that could not open is told', () => {
  it.each([
    // A closed connection and a protocol violation: reopening may work.
    ['Sync disconnected', 'retry'],
    ['RESYNC_REQUIRED', 'retry'],
    ['INVALID', 'retry'],
    ['GAP', 'retry'],
    ['CONFLICT', 'retry'],
    ['REMOTE_STATE_UNAVAILABLE', 'retry'],
    ['REMOTE_CORE_UNAVAILABLE', 'retry'],
    ['REMOTE_SERVE_STOPPED', 'retry'],
    ['Remote session ended', 'retry'],
    ['REMOTE_SESSION_ENDED', 'retry'],
    ['REMOTE_SEQUENCE_UNAVAILABLE', 'retry'],
    // The epoch advanced or the grant moved: a fresh open binds the new one.
    ['STALE_EPOCH', 'retry'],
    ['DENIED', 'retry'],
    ['EXPIRED', 'retry'],
    // Nobody classified it: a Retry costs nothing and the token stays visible.
    ['SURPRISE_CODE', 'retry'],
    // Reopening cannot make a page smaller.
    ['CAPACITY', 'large'],
    ['Sync message capacity', 'large'],
    ['REMOTE_INPUT_TOO_LARGE', 'large'],
    // A refusal that ends access, or a page that is gone.
    ['Access ended', 'refused'],
    ['REMOTE_SCOPE_DENIED', 'refused'],
    ['REMOTE_DEVICE_REVOKED', 'refused'],
    ['REMOTE_DEVICE_NOT_FOUND', 'refused'],
    ['Page unavailable', 'refused'],
  ])('%s is %s', (code, kind) => {
    const failure = openFailure(new Error(code));
    expect(failure.kind).toBe(kind);
    if (failure.kind !== 'refused') expect(failure.reference).toBe(code);
  });

  it('decides every code the live path knows, so a new code cannot ship undecided', () => {
    for (const code of Object.keys(CONNECTION_CODES))
      expect(['retry', 'large', 'refused']).toContain(openFailure(new Error(code)).kind);
  });

  it.each([
    ['unreachable', 'retry'],
    ['unconfirmed', 'retry'],
    ['transient', 'retry'],
    ['capacity', 'retry'],
    ['unpaired', 'refused'],
    ['mismatch', 'refused'],
    ['revoked', 'refused'],
    ['expired', 'refused'],
  ] as const)('a fresh open that failed as %s is %s', (reason, kind) => {
    expect(openFailure(new ReopenFailedError(reason)).kind).toBe(kind);
  });

  it('keeps the session limit and a named refusal of the admission as refusals', () => {
    expect(openFailure(new SessionLimitError(2)).kind).toBe('refused');
    expect(openFailure(new AdmissionRefusedError(403, '', 'REMOTE_DEVICE_REVOKED')).kind).toBe(
      'refused',
    );
    expect(openFailure(new AdmissionRefusedError(403, '', 'REMOTE_CLOSED')).kind).toBe('refused');
    // The mount answering for an extension that is not there says nothing about the device.
    expect(openFailure(new AdmissionRefusedError(503, '')).kind).toBe('retry');
  });

  it('shows a message that is already a sentence as a refusal, with no reference to keep', () => {
    expect(openFailure(new Error(text.noWraps))).toEqual({ kind: 'refused' });
    expect(openFailure(new Error(text.reconnectFailed))).toEqual({ kind: 'refused' });
  });

  it('keeps the reference bounded', () => {
    const failure = openFailure(new Error('x'.repeat(500)));
    expect(failure.kind).toBe('retry');
    if (failure.kind === 'retry') expect(failure.reference).toHaveLength(120);
  });
});
