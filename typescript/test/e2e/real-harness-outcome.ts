/** Public-resource assertion for #5619, not a provider launcher or completion detector. */
type Provider = 'claude' | 'codex';
type FailureCode = 'worker_error' | 'budget_timeout' | 'storage_error';

export interface CapturedFinal {
  responseId: string;
  text: string;
  provider: string;
  sessionId?: string;
}

export interface ConversationObservation {
  timeline: {
    identityId: string;
    capture?: { enabled: boolean };
    entries: {
      entryId: string;
      kind: string;
      inputBatchId?: string;
      availability?: string;
      responseId?: string;
      failure?: { code: string };
    }[];
    responses: CapturedFinal[];
  };
  recent: {
    identityId: string;
    capture?: { enabled: boolean };
    items: CapturedFinal[];
  };
}

export interface TextTurnEvidence {
  identityId: string;
  provider: Provider;
  sessionId: string;
  prompt: string;
  responseMarker: string;
  admitted?: { identityId: string; provider: Provider; sessionId: string };
  textBearingEnd?: { provider: Provider; sessionId: string };
}

export type ConversationOutcome =
  | { kind: 'retained'; entryId: string; responseId: string }
  | { kind: 'not_captured'; entryId: string; code: FailureCode };

function precondition(holds: boolean, reason: string): asserts holds {
  if (!holds) throw new Error(`CAPTURE_PRECONDITION_FAILED: ${reason}`);
}

/**
 * The caller owns a private, isolated single text turn, its public baseline and the
 * qualified admission/end evidence. A served mock response, idle pane or process exit
 * supplies none of that evidence. This offline slice does not qualify the real seam.
 *
 * Call with public responses.timeline/recent observations after the finite flush
 * allowance. This assertion neither polls nor treats a transient absence as completion.
 * IDs are opaque: only equality against the before-turn baseline is meaningful.
 */
export function assertConversationOutcome(
  turn: TextTurnEvidence,
  before: ConversationObservation,
  after: ConversationObservation
): ConversationOutcome {
  precondition(Boolean(turn.identityId && turn.sessionId), 'identity/session is missing');
  precondition(
    Boolean(turn.responseMarker) &&
      turn.responseMarker.length <= 128 &&
      !turn.prompt.includes(turn.responseMarker),
    'response marker must be bounded and absent from the prompt'
  );
  precondition(
    turn.admitted?.identityId === turn.identityId &&
      turn.admitted.provider === turn.provider &&
      turn.admitted.sessionId === turn.sessionId,
    'the expected identity/provider/session was not admitted'
  );
  precondition(
    turn.textBearingEnd?.provider === turn.provider &&
      turn.textBearingEnd.sessionId === turn.sessionId,
    'a qualified text-bearing end for this session is missing'
  );
  for (const observation of [before, after]) {
    precondition(
      observation.timeline.identityId === turn.identityId &&
        observation.recent.identityId === turn.identityId,
      'public observations belong to another identity'
    );
    precondition(
      observation.timeline.capture?.enabled === true &&
        observation.recent.capture?.enabled === true,
      'capture is not enabled in both public resources'
    );
  }

  const previousEntries = new Set(before.timeline.entries.map((entry) => entry.entryId));
  const previousResponses = new Set([
    ...before.timeline.responses.map((item) => item.responseId),
    ...before.recent.items.map((item) => item.responseId),
    ...before.timeline.entries.flatMap((entry) => (entry.responseId ? [entry.responseId] : [])),
  ]);
  for (const entry of after.timeline.entries) {
    if (!entry.entryId || previousEntries.has(entry.entryId)) continue;
    if (entry.kind !== 'final' || entry.availability !== 'retained' || !entry.responseId) continue;
    if (previousResponses.has(entry.responseId)) continue;
    const body = after.timeline.responses.find((item) => item.responseId === entry.responseId);
    const recent = after.recent.items.find((item) => item.responseId === entry.responseId);
    if (
      body?.provider === turn.provider &&
      body.sessionId === turn.sessionId &&
      body.text.includes(turn.responseMarker) &&
      recent?.provider === body.provider &&
      recent.sessionId === body.sessionId &&
      recent.text === body.text
    ) {
      return { kind: 'retained', entryId: entry.entryId, responseId: entry.responseId };
    }
  }
  for (const entry of after.timeline.entries) {
    if (!entry.entryId || previousEntries.has(entry.entryId) || entry.kind !== 'failure') continue;
    const code = entry.failure?.code;
    if (code === 'worker_error' || code === 'budget_timeout' || code === 'storage_error') {
      // An unbatched failure is valid, but claims no input or request association.
      return { kind: 'not_captured', entryId: entry.entryId, code };
    }
  }
  // Test-only label, not a new product error. Never dump response/prompt bodies.
  throw new Error(
    `CAPTURE_OUTCOME_MISSING: ${turn.provider}; public entries=${after.timeline.entries.length}, ` +
      `bodies=${after.timeline.responses.length}, recent=${after.recent.items.length}; ` +
      `entryIds=${JSON.stringify(after.timeline.entries.slice(0, 8).map((entry) => entry.entryId.slice(0, 80)))}`
  );
}
