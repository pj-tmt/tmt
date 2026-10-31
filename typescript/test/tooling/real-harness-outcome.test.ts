import { afterEach, describe, expect, it } from 'vite-plus/test';
import {
  assertConversationOutcome,
  type ConversationObservation,
  type TextTurnEvidence,
} from '../e2e/real-harness-outcome.js';
import { MOCK_MODEL_TOKEN, startMockModelServer } from '../support/mock-model-server.mjs';

const identityId = '6d380799-1725-4fe8-9842-602a5f8d087a';
const marker = 'T4_SYNTHETIC_RESPONSE_83c4';
const text = `Synthetic reply: ${marker} 雪`;
const servers: Awaited<ReturnType<typeof startMockModelServer>>[] = [];
afterEach(async () => {
  await Promise.all(servers.splice(0).map((server) => server.close()));
});

function turn(provider: 'claude' | 'codex' = 'claude'): TextTurnEvidence {
  const sessionId = `fixture-${provider}-session`;
  return {
    identityId,
    provider,
    sessionId,
    prompt: 'Give the scripted synthetic response.',
    responseMarker: marker,
    admitted: { identityId, provider, sessionId },
    textBearingEnd: { provider, sessionId },
  };
}

function empty(): ConversationObservation {
  return {
    timeline: { identityId, capture: { enabled: true }, entries: [], responses: [] },
    recent: { identityId, capture: { enabled: true }, items: [] },
  };
}

function retained(provider: 'claude' | 'codex' = 'claude'): ConversationObservation {
  const body = {
    responseId: 'opaque-response-a',
    inputBatchId: 'batch-a',
    provider,
    sessionId: `fixture-${provider}-session`,
    text,
    endedMs: 123,
    truncated: false,
    originalBytes: Buffer.byteLength(text),
  };
  return {
    timeline: {
      ...empty().timeline,
      entries: [
        {
          entryId: 'opaque-entry-a',
          inputBatchId: 'batch-a',
          kind: 'final',
          availability: 'retained',
          responseId: body.responseId,
        },
      ],
      responses: [body],
    },
    recent: { ...empty().recent, items: [{ ...body }] },
  };
}

function failure(code = 'worker_error', batched = true): ConversationObservation {
  const observation = empty();
  observation.timeline.entries.push({
    entryId: 'opaque-failure-a',
    ...(batched ? { inputBatchId: 'batch-a' } : {}),
    kind: 'failure',
    failure: { code },
  });
  return observation;
}

const missing = /^CAPTURE_OUTCOME_MISSING:/;
const precondition = /^CAPTURE_PRECONDITION_FAILED:/;

describe('offline real-harness conversation outcome assertion', () => {
  it.each(['claude', 'codex'] as const)(
    'accepts a retained %s final in both public views',
    (provider) => {
      expect(assertConversationOutcome(turn(provider), empty(), retained(provider))).toEqual({
        kind: 'retained',
        entryId: 'opaque-entry-a',
        responseId: 'opaque-response-a',
      });
    }
  );

  it.each(['worker_error', 'budget_timeout', 'storage_error'])(
    'accepts an existing visible %s outcome',
    (code) => {
      expect(assertConversationOutcome(turn(), empty(), failure(code))).toEqual({
        kind: 'not_captured',
        entryId: 'opaque-failure-a',
        code,
      });
    }
  );

  it('accepts an unbatched failure without inventing a prompt or request link', () => {
    const outcome = assertConversationOutcome(turn(), empty(), failure('worker_error', false));
    expect(outcome).toEqual({
      kind: 'not_captured',
      entryId: 'opaque-failure-a',
      code: 'worker_error',
    });
    expect(outcome).not.toHaveProperty('inputBatchId');
    expect(outcome).not.toHaveProperty('requestId');
  });

  it('fails when neither terminal outcome exists', () => {
    expect(() => assertConversationOutcome(turn(), empty(), empty())).toThrow(missing);
  });

  it('does not reuse old finals or failures from the baseline', () => {
    for (const before of [retained(), failure()]) {
      expect(() => assertConversationOutcome(turn(), before, structuredClone(before))).toThrow(
        missing
      );
    }
    const before = empty();
    before.recent.items = retained().recent.items;
    expect(() => assertConversationOutcome(turn(), before, retained())).toThrow(missing);
  });

  it('rejects a new reference to an already observed response ID', () => {
    const before = retained();
    const after = retained();
    after.timeline.entries[0].entryId = 'another-opaque-entry';
    expect(() => assertConversationOutcome(turn(), before, after)).toThrow(missing);
  });

  it.each(['timeline', 'recent'] as const)('rejects another identity in %s', (view) => {
    for (const stage of ['before', 'after'] as const) {
      const before = empty();
      const after = retained();
      (stage === 'before' ? before : after)[view].identityId = 'another-identity';
      expect(() => assertConversationOutcome(turn(), before, after)).toThrow(precondition);
    }
  });

  it('rejects missing or mismatched admission and end evidence, even with a failure', () => {
    const admitted = { identityId, provider: 'claude' as const, sessionId: turn().sessionId };
    const cases: TextTurnEvidence[] = [
      { ...turn(), admitted: undefined },
      { ...turn(), admitted: { ...admitted, identityId: 'another-identity' } },
      { ...turn(), admitted: { ...admitted, sessionId: 'another-session' } },
      { ...turn(), admitted: { ...admitted, provider: 'codex' } },
      { ...turn(), textBearingEnd: undefined },
      { ...turn(), textBearingEnd: { provider: 'claude', sessionId: 'another-session' } },
      { ...turn(), textBearingEnd: { provider: 'codex', sessionId: turn().sessionId } },
    ];
    for (const evidence of cases) {
      expect(() => assertConversationOutcome(evidence, empty(), failure())).toThrow(precondition);
    }
  });

  it.each(['timeline', 'recent'] as const)(
    'requires enabled capture in %s before and after',
    (view) => {
      for (const stage of ['before', 'after'] as const) {
        const before = empty();
        const after = retained();
        (stage === 'before' ? before : after)[view].capture = undefined;
        expect(() => assertConversationOutcome(turn(), before, after)).toThrow(precondition);
        (stage === 'before' ? before : after)[view].capture = { enabled: false };
        expect(() => assertConversationOutcome(turn(), before, after)).toThrow(precondition);
      }
    }
  );

  it('rejects empty, unbounded or prompt-echo markers', () => {
    for (const responseMarker of ['', 'x'.repeat(129), turn().prompt]) {
      expect(() =>
        assertConversationOutcome({ ...turn(), responseMarker }, empty(), retained())
      ).toThrow(precondition);
    }
  });

  it('rejects unavailable bodies, unrelated text, wrong sessions and inconsistent recent views', () => {
    const mutations: ((value: ConversationObservation) => void)[] = [
      (value) => {
        value.timeline.entries[0].availability = 'unavailable';
      },
      (value) => {
        value.timeline.responses = [];
      },
      (value) => {
        value.recent.items = [];
      },
      (value) => {
        value.timeline.responses[0].text = 'unrelated final';
      },
      (value) => {
        value.timeline.responses[0].sessionId = 'another-session';
      },
      (value) => {
        value.timeline.responses[0].provider = 'codex';
      },
      (value) => {
        value.recent.items[0].text = 'inconsistent body';
      },
      (value) => {
        value.recent.items[0].sessionId = 'another-session';
      },
      (value) => {
        value.recent.items[0].responseId = 'another-response';
      },
      (value) => {
        value.timeline.entries[0].entryId = '';
      },
    ];
    for (const mutate of mutations) {
      const observation = retained();
      mutate(observation);
      expect(() => assertConversationOutcome(turn(), empty(), observation)).toThrow(missing);
    }
  });

  it('does not substitute an explicit reply, input echo or unknown failure code for an outcome', () => {
    for (const kind of ['reply', 'input']) {
      const observation = retained();
      observation.timeline.entries[0].kind = kind;
      expect(() => assertConversationOutcome(turn(), empty(), observation)).toThrow(missing);
    }
    expect(() => assertConversationOutcome(turn(), empty(), failure('invented_code'))).toThrow(
      missing
    );
  });

  it('fails the named invariant when only the terminal outcome is removed', () => {
    const positive = retained();
    expect(assertConversationOutcome(turn(), empty(), positive).kind).toBe('retained');
    const negative = structuredClone(positive);
    negative.timeline.entries = [];
    negative.timeline.responses = [];
    negative.recent.items = [];
    expect(() => assertConversationOutcome(turn(), empty(), negative)).toThrow(missing);
    expect(assertConversationOutcome(turn(), empty(), positive).kind).toBe('retained');
  });

  it.each(['messages', 'responses'] as const)(
    'uses the existing %s mock transport without mistaking served text for capture',
    async (protocol) => {
      const server = await startMockModelServer({
        turns: [{ protocol, model: 'fixture-model', text, inputTokens: 2, outputTokens: 8 }],
        limits: { maxRequests: 1, timeoutMs: 5000 },
      });
      servers.push(server);
      const response = await fetch(`${server.baseUrl}/v1/${protocol}`, {
        method: 'POST',
        headers: {
          authorization: `Bearer ${MOCK_MODEL_TOKEN}`,
          'content-type': 'application/json',
        },
        body: JSON.stringify({
          model: 'fixture-model',
          stream: false,
          ...(protocol === 'messages'
            ? { messages: [{ role: 'user', content: turn().prompt }] }
            : { input: [{ role: 'user', content: turn().prompt }] }),
        }),
        signal: AbortSignal.timeout(2000),
      });
      expect(response.status).toBe(200);
      const served: unknown = await response.json();
      expect(served).toMatchObject(
        protocol === 'messages'
          ? { content: [{ type: 'text', text }] }
          : { output: [{ content: [{ type: 'output_text', text }] }] }
      );
      expect(server.snapshot().completedTurns).toBe(1);
      expect(server.snapshot()).not.toHaveProperty('failure');
      // Synthetic end evidence tests the checker only; HTTP completion is not its source.
      const evidence = turn(protocol === 'messages' ? 'claude' : 'codex');
      expect(() => assertConversationOutcome(evidence, empty(), empty())).toThrow(missing);
      expect(assertConversationOutcome(evidence, empty(), retained(evidence.provider)).kind).toBe(
        'retained'
      );
      await server.close();
      await server.closed;
      expect(server.snapshot().closed).toBe(true);
    }
  );
});
