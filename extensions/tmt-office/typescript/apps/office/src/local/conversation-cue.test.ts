import { expect, it } from 'vitest';
import { conversationCue } from './conversation-cue.js';
import type { ConversationState } from './conversation-state.js';
import type { HistorySummary } from './request-history-contract.js';

const requestId = 'req_22222222-2222-4222-8222-222222222222';
const item: HistorySummary = {
  requestId,
  recipientId: '11111111-1111-4111-8111-111111111111',
  roomId: null,
  sender: { kind: 'unknown', identityId: null },
  kind: 'request',
  preparedAtMs: 1000,
  delivery: 'queued',
  recipientAcknowledged: false,
  final: { status: 'not_submitted' },
  preview: 'Question',
};
const state: ReturnType<ConversationState['getSnapshot']> = {
  page: { items: [item], nextBefore: null },
  details: {},
  detailErrors: {},
  loading: false,
  paused: false,
};
const withItem = (change: Partial<HistorySummary>) => ({
  ...state,
  page: { items: [{ ...item, ...change }], nextBefore: null },
});

it('does not mistake delivery or acknowledgment for execution or completion', () => {
  for (const delivery of ['sent', 'queued'] as const) {
    for (const recipientAcknowledged of [true, false]) {
      expect(conversationCue(withItem({ delivery, recipientAcknowledged }))).toEqual({
        kind: 'waiting',
        text: 'Awaiting reply',
        requestId,
      });
    }
  }
  expect(conversationCue(withItem({ delivery: 'uncertain' })).text).toBe('Delivery uncertain');
  expect(conversationCue(withItem({ delivery: 'definitely_failed' })).text).toBe('Not delivered');
  expect(conversationCue(withItem({ final: { status: 'not_required' } })).text).toBe(
    'No reply requested'
  );
});

it('uses retained reply metadata and a view-local seen marker, never writes an ack', () => {
  const reply = withItem({
    final: {
      status: 'retained',
      submittedAtMs: 2000,
      expiresAtMs: 10000,
      bodyBytes: 6,
      content: undefined,
    },
  });
  expect(conversationCue(reply)).toEqual({ kind: 'reply', text: 'Reply ready', requestId });
  expect(conversationCue(reply, requestId, [requestId])).toEqual({
    kind: 'idle',
    text: 'Reply received',
    requestId,
  });
  for (const status of ['expired', 'unavailable'] as const) {
    expect(
      conversationCue(withItem({ final: { status, submittedAtMs: 2000, expiresAtMs: 10000 } }))
    ).toEqual({ kind: 'notice', text: `Reply ${status}`, requestId });
  }
});

it('does not attach another request or a stale observation to the latest submission', () => {
  expect(conversationCue(state, 'different-request')).toEqual({
    kind: 'idle',
    text: 'Request submitted',
  });
  expect(conversationCue({ ...state, page: undefined })).toEqual({
    kind: 'idle',
    text: 'Open chat',
  });
  expect(conversationCue({ ...state, paused: true }).text).toBe('Updates paused');
  expect(conversationCue({ ...state, error: 'offline' }).text).toBe('Could not check reply');
  expect(conversationCue({ ...state, before: { preparedAtMs: 1000, requestId } }).text).toBe(
    'Viewing earlier messages'
  );
});

it('prioritizes any unseen reply in the bounded window over a newer pending request', () => {
  const newerId = 'req_33333333-3333-4333-8333-333333333333';
  const olderReply: HistorySummary = {
    ...item,
    final: {
      status: 'retained',
      submittedAtMs: 2000,
      expiresAtMs: 10000,
      bodyBytes: 6,
      content: undefined,
    },
  };
  const window = {
    ...state,
    page: { items: [{ ...item, requestId: newerId }, olderReply], nextBefore: null },
  };
  expect(conversationCue(window, newerId)).toEqual({
    kind: 'reply',
    text: 'Reply ready',
    requestId,
  });
  expect(conversationCue(window, newerId, [requestId])).toEqual({
    kind: 'waiting',
    text: 'Awaiting reply',
    requestId: newerId,
  });
  expect(conversationCue(window, 'displaced-request')).toEqual({
    kind: 'reply',
    text: 'Reply ready',
    requestId,
  });
});
