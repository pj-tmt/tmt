import { describe, expect, it } from 'vite-plus/test';
import { REMOTE_REFUSAL_CODES } from '../src/ask-remote.js';
import { SYNC_ERROR_CODES } from '../src/connection.js';
import { text } from '../src/strings.js';
import { FAILURE_CAUSES, terminalFailure } from '../src/terminal-failure.js';

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
    for (const code of Object.keys(FAILURE_CAUSES))
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
    expect(terminalFailure(new Error(text.noWraps))).toEqual({ sentence: text.noWraps });
  });

  // A code added to the wire or Remote lists must get a wording decision here,
  // instead of shipping to readers as a raw headline.
  it('has a decision for every sync frame code and Remote refusal code', () => {
    for (const code of [...SYNC_ERROR_CODES, ...REMOTE_REFUSAL_CODES]) {
      // Eviction has its own card with the session limit and the command to run.
      if (code === 'REMOTE_SESSION_EVICTED') continue;
      expect(FAILURE_CAUSES, code).toHaveProperty(code);
    }
  });
});
