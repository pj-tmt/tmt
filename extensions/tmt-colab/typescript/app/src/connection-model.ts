import { SessionEvictedError } from './ask-remote.js';
import { classify, type FailureCause } from './connection-state.js';
import { RecoveryRequiredError } from './session-recovery.js';

/** Why a page's connection ended for good: a failure cause, or the session limit. */
export type EndedReason = FailureCause | 'evicted';

/** The one thing the reader can do about an ended connection. */
export type EndedAction = 'pair' | 'pages' | 'none';

/**
 * What a failure the page publishes becomes (#2572):
 * - `reconnecting`: silent, drafts and the open thread kept. `transport` is a lost tunnel or
 *   session that a fresh open can replace; `epoch` is an epoch advance that reopens the page.
 * - `ended`: one reason and one action; recovery is exhausted or was never possible.
 */
export type Settled =
  | { readonly phase: 'reconnecting'; readonly kind: 'transport' | 'epoch' }
  | { readonly phase: 'ended'; readonly reason: EndedReason; readonly action: EndedAction };

export type Ended = Extract<Settled, { phase: 'ended' }>;

/** A page connection as every reader sees it; `error` is the failure behind a state that is not connected. */
export type ConnectionState =
  | { readonly phase: 'connected' }
  | (Extract<Settled, { phase: 'reconnecting' }> & { readonly error: Error })
  | (Ended & { readonly error: Error });

const ACTIONS: Record<FailureCause, EndedAction> = {
  // Access ended or the pairing is gone: only a new pairing helps.
  access: 'pair',
  unpaired: 'pair',
  // Nothing to pair for a page that no longer exists.
  gone: 'pages',
  session: 'none',
  large: 'none',
  generic: 'none',
};

/** The ended state of a failure, whatever its classification says about recovery. */
export function terminal(error: Error): Ended {
  if (error instanceof SessionEvictedError)
    return { phase: 'ended', reason: 'evicted', action: 'none' };
  const { cause } = classify(error);
  return { phase: 'ended', reason: cause, action: ACTIONS[cause] };
}

/** The single decision for a failure the page publishes; callers never read raw messages. */
export function settle(error: Error): Settled {
  if (error instanceof RecoveryRequiredError) return { phase: 'reconnecting', kind: 'transport' };
  const { recovery } = classify(error);
  if (recovery === 'reopen') return { phase: 'reconnecting', kind: 'epoch' };
  if (recovery === 'retry') return { phase: 'reconnecting', kind: 'transport' };
  return terminal(error);
}
