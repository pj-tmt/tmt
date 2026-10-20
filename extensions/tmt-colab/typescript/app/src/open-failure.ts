import { classify } from './connection-state.js';
import { SessionLimitError } from './session-limit-error.js';
import { terminalFailure } from './terminal-failure.js';

/**
 * How a page that could not open is told to the reader. The open has no deadline (#3975), so it
 * fails only by a closed connection, a protocol violation or a refusal:
 * - `retry`: nothing says reopening cannot work (a closed connection, a protocol violation, an
 *   ended Remote session, an unclassified failure). Plain copy, a Retry action and the raw token
 *   as a small reference.
 * - `large`: the page is over what the browser reads, so reopening cannot help. Plain copy, no
 *   Retry.
 * - `refused`: access ended, the browser is not paired, the page is gone or the session limit is
 *   reached. The red card, with its own sentence.
 */
export type OpenFailure =
  | { kind: 'retry'; reference: string }
  | { kind: 'large'; reference: string }
  | { kind: 'refused' };

export function openFailure(error: Error): OpenFailure {
  if (error instanceof SessionLimitError) return { kind: 'refused' };
  // A message that is already a sentence for the reader has no reference to show: it is a refusal.
  const { reference } = terminalFailure(error);
  if (!reference) return { kind: 'refused' };
  const { recovery, cause } = classify(error);
  if (recovery !== 'none') return { kind: 'retry', reference };
  switch (cause) {
    case 'access':
    case 'pairing':
    case 'gone':
    case 'unpaired':
    case 'capacity':
      return { kind: 'refused' };
    case 'large':
      return { kind: 'large', reference };
    case 'session':
    case 'generic':
      return { kind: 'retry', reference };
  }
}
