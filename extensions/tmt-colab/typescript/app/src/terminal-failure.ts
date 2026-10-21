import { classify, type FailureCause } from './connection-state.js';
import { text } from './strings.js';
import { AdmissionRefusedError } from './registration.js';

interface FailurePresentation {
  sentence: string;
  state: 'blocked' | 'inactive';
}
const PRESENTATIONS: Record<FailureCause, FailurePresentation> = {
  access: { sentence: text.failureAccessEnded, state: 'blocked' },
  revoked: { sentence: text.failurePairingRemoved, state: 'blocked' },
  expired: { sentence: text.failurePairingExpired, state: 'blocked' },
  mismatch: { sentence: text.failureAccessEnded, state: 'blocked' },
  unpaired: { sentence: text.failureUnpaired, state: 'blocked' },
  gone: { sentence: text.failurePageGone, state: 'inactive' },
  session: { sentence: text.failureSessionEnded, state: 'inactive' },
  large: { sentence: text.failureTooLarge, state: 'inactive' },
  capacity: { sentence: text.failureSessionLimit, state: 'inactive' },
  generic: { sentence: text.failureGeneric, state: 'inactive' },
};

/** A terminal sentence/state and readable label; retain bounded raw detail for diagnostics. */
export function terminalFailure(error: Error): FailurePresentation & {
  reference?: string;
  referenceLabel?: string;
} {
  if (error.message === text.noWraps) return { sentence: error.message, state: 'blocked' };
  if (error.message === text.reconnectFailed) return { sentence: error.message, state: 'inactive' };
  const classification = classify(error);
  const presentation =
    error instanceof AdmissionRefusedError && classification.recovery === 'reopen'
      ? { sentence: text.reconnectFailed, state: 'inactive' as const }
      : PRESENTATIONS[classification.cause];
  return {
    ...presentation,
    reference: error.message.slice(0, 120),
    referenceLabel: text.failureReference[classification.cause],
  };
}
