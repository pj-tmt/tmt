import type { AgentDestination } from './live-ask.js';

export type RecipientKey = Pick<AgentDestination, 'machine' | 'agent'>;
export type MessageRecipientDecision =
  | { kind: 'blocked' }
  | { kind: 'comment' }
  | { kind: 'agent'; destination: AgentDestination }
  | { kind: 'choose' }
  | { kind: 'unavailable' };

function matches(candidate: RecipientKey, key: RecipientKey) {
  return candidate.machine === key.machine && candidate.agent === key.agent;
}

/** Parent presentation policy, not Remote admission. No message bytes or effects enter this decision. */
export function messageRecipient({
  writable,
  intent,
  destinations,
  selected,
  replyRecipients = [],
  lastReplier,
}: {
  writable: boolean;
  intent: 'comment' | 'agent';
  /** Undefined means discovery failed or has not completed; [] is a successful empty result. */
  destinations?: readonly AgentDestination[];
  selected?: RecipientKey;
  /** Stable keys supplied by the parent's existing admitted conversation projection. */
  replyRecipients?: readonly RecipientKey[];
  lastReplier?: RecipientKey;
}): MessageRecipientDecision {
  if (!writable) return { kind: 'blocked' };
  if (intent === 'comment') return { kind: 'comment' };
  if (!destinations) return { kind: 'unavailable' };
  const eligible = destinations.filter(
    (candidate) => candidate.online === 'online' && candidate.presence !== 'offline',
  );
  function byKey(key: RecipientKey): MessageRecipientDecision {
    const candidates = eligible.filter((candidate) => matches(candidate, key));
    return candidates.length === 1
      ? { kind: 'agent', destination: candidates[0] }
      : { kind: 'unavailable' };
  }
  if (selected) return byKey(selected);
  const bound = replyRecipients.filter(
    (key, index) => replyRecipients.findIndex((candidate) => matches(candidate, key)) === index,
  );
  if (bound.length > 1) return { kind: 'choose' };
  if (bound.length === 1) return byKey(bound[0]);
  if (lastReplier) return byKey(lastReplier);
  return { kind: 'choose' };
}
