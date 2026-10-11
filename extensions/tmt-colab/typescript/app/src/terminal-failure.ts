import { classify, type FailureCause } from './connection-state.js';
import { text } from './strings.js';

const SENTENCES: Record<FailureCause, string> = {
  access: text.failureAccessEnded,
  unpaired: text.failureUnpaired,
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
  return { sentence: SENTENCES[classify(error).cause], reference: error.message.slice(0, 120) };
}
