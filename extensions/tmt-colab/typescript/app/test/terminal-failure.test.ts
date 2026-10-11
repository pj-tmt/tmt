import { describe, expect, it } from 'vite-plus/test';
import { CONNECTION_CODES } from '../src/connection-state.js';
import { ReopenFailedError } from '../src/registration.js';
import { text } from '../src/strings.js';
import { terminalFailure } from '../src/terminal-failure.js';

describe('terminal failure wording', () => {
  it.each([
    ['DENIED', text.failureAccessEnded],
    ['STALE_EPOCH', text.failureAccessEnded],
    ['Access ended', text.failureAccessEnded],
    ['REMOTE_DEVICE_REVOKED', text.failureAccessEnded],
    // The one silent re-admission was refused, so the close is the access ending (#2619).
    ['REMOTE_CLOSED', text.failureAccessEnded],
    ['REMOTE_GRANT_CHANGED', text.failureAccessEnded],
    ['REMOTE_SERVE_STOPPED', text.failureGeneric],
    ['Page unavailable', text.failurePageGone],
    ['REMOTE_SESSION_ENDED', text.failureSessionEnded],
    ['CAPACITY', text.failureTooLarge],
    ['REMOTE_INPUT_TOO_LARGE', text.failureTooLarge],
  ])('%s becomes a sentence and keeps the code as the reference', (code, sentence) => {
    expect(terminalFailure(new Error(code))).toEqual({ sentence, reference: code });
  });

  it.each([
    ['revoked', text.failureAccessEnded],
    ['expired', text.failureAccessEnded],
    ['unpaired', text.failureUnpaired],
    ['mismatch', text.failureUnpaired],
    ['capacity', text.failureGeneric],
  ] as const)('a fresh open refused as %s ends with its own sentence', (reason, sentence) => {
    expect(terminalFailure(new ReopenFailedError(reason))).toEqual({
      sentence,
      reference: `REMOTE_REOPEN_${reason.toUpperCase()}`,
    });
  });

  it('never makes a raw code the sentence', () => {
    for (const code of Object.keys(CONNECTION_CODES))
      expect(terminalFailure(new Error(code)).sentence).not.toBe(code);
  });

  it('words an unknown code generically and keeps it visible, bounded', () => {
    expect(terminalFailure(new Error('SURPRISE_CODE'))).toEqual({
      sentence: text.failureGeneric,
      reference: 'SURPRISE_CODE',
    });
    expect(terminalFailure(new Error('x'.repeat(500))).reference).toHaveLength(120);
  });

  it('shows a message that is already a sentence as is, without a reference', () => {
    for (const sentence of [text.noWraps, text.reconnectFailed])
      expect(terminalFailure(new Error(sentence))).toEqual({ sentence });
  });
});
