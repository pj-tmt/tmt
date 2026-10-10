import { describe, expect, it } from 'vite-plus/test';
import { CONNECTION_CODES } from '../src/connection-state.js';
import { text } from '../src/strings.js';
import { terminalFailure } from '../src/terminal-failure.js';

describe('terminal failure wording', () => {
  it.each([
    ['DENIED', text.failureAccessEnded],
    ['STALE_EPOCH', text.failureAccessEnded],
    ['Access ended', text.failureAccessEnded],
    ['REMOTE_DEVICE_REVOKED', text.failureAccessEnded],
    ['Page unavailable', text.failurePageGone],
    ['REMOTE_SESSION_ENDED', text.failureSessionEnded],
    ['CAPACITY', text.failureTooLarge],
    ['REMOTE_INPUT_TOO_LARGE', text.failureTooLarge],
  ])('%s becomes a sentence and keeps the code as the reference', (code, sentence) => {
    expect(terminalFailure(new Error(code))).toEqual({ sentence, reference: code });
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
