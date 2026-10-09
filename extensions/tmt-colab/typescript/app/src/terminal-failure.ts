import { text } from './strings.js';

/** Why a live page ended for good. Each cause owns one `strings.ts` sentence. */
export type FailureCause = 'access' | 'gone' | 'session' | 'large' | 'generic';

/**
 * Every raw token the live path can end a page with, decided once. A code absent
 * here is unknown: it gets the generic sentence and stays visible as the reference.
 */
export const FAILURE_CAUSES: Readonly<Record<string, FailureCause>> = {
  DENIED: 'access',
  EXPIRED: 'access',
  STALE_EPOCH: 'access',
  'Access ended': 'access',
  REMOTE_SCOPE_DENIED: 'access',
  REMOTE_CLOSED: 'access',
  REMOTE_DEVICE_REVOKED: 'access',
  REMOTE_DEVICE_NOT_FOUND: 'access',
  'Page unavailable': 'gone',
  REMOTE_SESSION_ENDED: 'session',
  REMOTE_SEQUENCE_UNAVAILABLE: 'session',
  CAPACITY: 'large',
  'Sync message capacity': 'large',
  REMOTE_INPUT_TOO_LARGE: 'large',
  // Recoverable or internal refusals that reach the card only when recovery gave up.
  INVALID: 'generic',
  GAP: 'generic',
  CONFLICT: 'generic',
  RESYNC_REQUIRED: 'generic',
  REMOTE_INPUT_INVALID: 'generic',
  REMOTE_RATE_LIMITED: 'generic',
  REMOTE_REPLAY: 'generic',
  REMOTE_INTENT_CONFLICT: 'generic',
  REMOTE_STATE_UNAVAILABLE: 'generic',
  REMOTE_CORE_UNAVAILABLE: 'generic',
};

const SENTENCES: Record<FailureCause, string> = {
  access: text.failureAccessEnded,
  gone: text.failurePageGone,
  session: text.failureSessionEnded,
  large: text.failureTooLarge,
  generic: text.failureGeneric,
};

/** Messages that are already a sentence for the reader: shown as is, no reference. */
const SENTENCE_MESSAGES: readonly string[] = [text.noWraps, text.reconnectFailed];

/** The sentence a reader sees for a terminal failure, plus the raw token to keep as a reference. */
export function terminalFailure(error: Error): { sentence: string; reference?: string } {
  if (SENTENCE_MESSAGES.includes(error.message)) return { sentence: error.message };
  const cause = FAILURE_CAUSES[error.message];
  return { sentence: SENTENCES[cause ?? 'generic'], reference: error.message.slice(0, 120) };
}
