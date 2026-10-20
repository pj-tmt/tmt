import { describe, expect, it } from 'vite-plus/test';
import { CONNECTION_CODES } from '../src/connection-state.js';
import { ReopenFailedError } from '../src/registration.js';
import { SessionLimitError } from '../src/session-limit-error.js';
import { text } from '../src/strings.js';
import { terminalFailure } from '../src/terminal-failure.js';

describe('terminal failure wording', () => {
  it.each([
    ['DENIED', text.failureAccessEnded],
    ['STALE_EPOCH', text.failureAccessEnded],
    ['Access ended', text.failureAccessEnded],
    ['REMOTE_DEVICE_REVOKED', text.failurePairingRemoved],
    // The one silent re-admission was refused, so the close is the access ending (#2619).
    ['REMOTE_CLOSED', text.failureAccessEnded],
    ['REMOTE_GRANT_CHANGED', text.failureAccessEnded],
    ['REMOTE_SERVE_STOPPED', text.failureGeneric],
    ['Page unavailable', text.failurePageGone],
    ['REMOTE_SESSION_ENDED', text.failureSessionEnded],
    ['CAPACITY', text.failureTooLarge],
    ['REMOTE_INPUT_TOO_LARGE', text.failureTooLarge],
  ])('%s becomes a sentence and keeps the code as the reference', (code, sentence) => {
    expect(terminalFailure(new Error(code))).toMatchObject({ sentence, reference: code });
  });

  it.each([
    ['revoked', text.failurePairingRemoved],
    ['expired', text.failurePairingExpired],
    ['unpaired', text.failureUnpaired],
    ['mismatch', text.failureAccessEnded],
    ['capacity', text.failureGeneric],
  ] as const)('a fresh open refused as %s ends with its own sentence', (reason, sentence) => {
    expect(terminalFailure(new ReopenFailedError(reason))).toMatchObject({
      sentence,
      reference: `REMOTE_REOPEN_${reason.toUpperCase()}`,
    });
  });

  it('names the session limit for a refused open and keeps its code as the reference', () => {
    expect(terminalFailure(new SessionLimitError(2))).toMatchObject({
      sentence: text.failureSessionLimit,
      reference: 'REMOTE_SESSION_LIMIT',
    });
  });

  it.each([
    ['REMOTE_SCOPE_DENIED', 'blocked', 'Access denied'],
    ['REMOTE_DEVICE_REVOKED', 'blocked', 'Pairing removed'],
    ['REMOTE_GRANT_EXPIRED', 'blocked', 'Pairing expired'],
    ['REMOTE_DEVICE_NOT_FOUND', 'blocked', 'Not paired'],
    ['REMOTE_SESSION_ENDED', 'inactive', 'Session ended'],
    ['Page unavailable', 'inactive', 'Page unavailable'],
    ['SURPRISE_CODE', 'inactive', 'Connection unavailable'],
  ])('%s has a readable label and the appropriate refusal state', (code, state, referenceLabel) => {
    expect(terminalFailure(new Error(code))).toMatchObject({
      state,
      reference: code,
      referenceLabel,
    });
  });

  it('never makes a raw code the sentence', () => {
    for (const code of Object.keys(CONNECTION_CODES))
      expect(terminalFailure(new Error(code)).sentence).not.toBe(code);
  });

  it('words an unknown code generically and keeps it visible, bounded', () => {
    expect(terminalFailure(new Error('SURPRISE_CODE'))).toMatchObject({
      sentence: text.failureGeneric,
      reference: 'SURPRISE_CODE',
    });
    expect(terminalFailure(new Error('x'.repeat(500))).reference).toHaveLength(120);
  });

  it('shows a message that is already a sentence as is, without a reference', () => {
    for (const sentence of [text.noWraps, text.reconnectFailed])
      expect(terminalFailure(new Error(sentence))).toMatchObject({ sentence });
  });
});
