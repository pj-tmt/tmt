import { describe, expect, it } from 'vite-plus/test';
import { SessionEvictedError } from '../src/ask-remote.js';
import { settle, terminal } from '../src/connection-model.js';
import { ReopenFailedError } from '../src/registration.js';
import { SessionLimitError } from '../src/session-limit-error.js';
import { RecoveryRequiredError } from '../src/session-recovery.js';

const of = (message: string) => new Error(message);

// What a failure the page publishes becomes: still trying (silent), or ended with one reason and
// one action. This is the whole mapping #2572 agreed with Remote and UX.
describe('connection model: settle', () => {
  it.each([
    [
      new RecoveryRequiredError(of('Sync disconnected')),
      { phase: 'reconnecting', kind: 'transport' },
    ],
    [of('STALE_EPOCH'), { phase: 'reconnecting', kind: 'epoch' }],
    [of('DENIED'), { phase: 'reconnecting', kind: 'epoch' }],
    [of('EXPIRED'), { phase: 'reconnecting', kind: 'epoch' }],
    [new ReopenFailedError('unreachable'), { phase: 'reconnecting', kind: 'transport' }],
    [new ReopenFailedError('unconfirmed'), { phase: 'reconnecting', kind: 'transport' }],
    [new ReopenFailedError('transient'), { phase: 'reconnecting', kind: 'transport' }],
  ] as const)('%s keeps trying without a card', (error, expected) => {
    expect(settle(error)).toEqual(expected);
  });

  it.each([
    [of('Access ended'), 'access', 'reload'],
    [of('REMOTE_DEVICE_REVOKED'), 'revoked', 'pair'],
    [of('REMOTE_DEVICE_NOT_FOUND'), 'unpaired', 'pair'],
    [of('REMOTE_GRANT_EXPIRED'), 'expired', 'pair'],
    [of('REMOTE_SCOPE_DENIED'), 'access', 'reload'],
    // A published legacy close means the fresh open was refused: the one silent try is spent.
    [of('REMOTE_CLOSED'), 'access', 'reload'],
    [of('REMOTE_GRANT_CHANGED'), 'access', 'reload'],
    [of('REMOTE_SERVE_STOPPED'), 'generic', 'reload'],
    [new ReopenFailedError('revoked'), 'revoked', 'pair'],
    [new ReopenFailedError('expired'), 'expired', 'pair'],
    [new ReopenFailedError('unpaired'), 'unpaired', 'pair'],
    [new ReopenFailedError('mismatch'), 'mismatch', 'reload'],
    [new ReopenFailedError('capacity'), 'generic', 'reload'],
    [of('Page unavailable'), 'gone', 'pages'],
    [of('REMOTE_SESSION_ENDED'), 'session', 'reload'],
    [of('CAPACITY'), 'large', 'none'],
    [of('SURPRISE_CODE'), 'generic', 'reload'],
  ] as const)('%s ends as %s with the action %s', (error, reason, action) => {
    expect(settle(error)).toEqual({ phase: 'ended', reason, action });
  });

  it('ends a refused open at the session limit with a reload, never a retry', () => {
    expect(settle(new SessionLimitError(2, 'https://x.test/settings'))).toEqual({
      phase: 'ended',
      reason: 'capacity',
      action: 'reload',
    });
  });

  it('ends an eviction with its own reason and no pairing action', () => {
    expect(settle(new SessionEvictedError(8, 'https://x.test/settings'))).toEqual({
      phase: 'ended',
      reason: 'evicted',
      action: 'none',
    });
  });
});

describe('connection model: terminal', () => {
  // A refused fresh open or an exhausted budget cannot keep trying: the same error ends.
  it('ends what settle would retry', () => {
    expect(terminal(of('STALE_EPOCH'))).toEqual({
      phase: 'ended',
      reason: 'access',
      action: 'reload',
    });
    expect(terminal(new RecoveryRequiredError(of('Sync disconnected')))).toEqual({
      phase: 'ended',
      reason: 'generic',
      action: 'reload',
    });
    expect(terminal(new ReopenFailedError('transient'))).toEqual({
      phase: 'ended',
      reason: 'generic',
      action: 'reload',
    });
  });

  it('leaves an ended error as settle decided', () => {
    expect(terminal(of('Page unavailable'))).toEqual(settle(of('Page unavailable')));
  });
});
