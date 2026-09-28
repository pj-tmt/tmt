import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import {
  createConversationState,
  CONVERSATION_OBSERVATION_MS,
  CONVERSATION_POLL_MS,
  CONVERSATION_PAGE_SIZE,
} from './conversation-state.js';
import type { HistoryDetail, HistoryPage, RequestHistoryPort } from './request-history-contract.js';

const recipientId = '11111111-1111-4111-8111-111111111111';
const requestId = 'req_22222222-2222-4222-8222-222222222222';
const summary = {
  requestId,
  recipientId,
  roomId: null,
  sender: { kind: 'unknown' as const, identityId: null },
  kind: 'request' as const,
  preparedAtMs: 1000,
  delivery: 'queued' as const,
  recipientAcknowledged: false,
  final: { status: 'not_submitted' as const },
  preview: 'Question',
};
const detail: HistoryDetail = {
  ...summary,
  prompt: { status: 'retained', message: 'Question', messageBytes: 8, expiresAtMs: 10000 },
};
function fixture() {
  const port = {
    list: vi.fn(async (): Promise<HistoryPage> => ({ items: [summary], nextBefore: null })),
    show: vi.fn(async () => detail),
    receipt: vi.fn(),
  };
  const state = createConversationState(port, { recipientId });
  return { state, port };
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(1000);
});
afterEach(() => vi.useRealTimers());

it('polls only visible metadata and reads long content only on change or explicit refresh', async () => {
  const { state, port } = fixture();
  expect(port.list).not.toHaveBeenCalled();
  state.setActive(true);
  await vi.advanceTimersByTimeAsync(0);
  expect(state.getSnapshot().details[requestId]).toEqual(detail);
  await vi.advanceTimersByTimeAsync(CONVERSATION_POLL_MS * 3);
  expect(port.list).toHaveBeenCalledTimes(4);
  expect(port.show).toHaveBeenCalledTimes(1);
  port.list.mockResolvedValue({
    items: [
      {
        ...summary,
        final: {
          status: 'retained',
          submittedAtMs: 2000,
          expiresAtMs: 10000,
          bodyBytes: 6,
          content: undefined,
        },
      },
    ],
    nextBefore: null,
  });
  port.show.mockResolvedValue({
    ...detail,
    final: {
      status: 'retained',
      submittedAtMs: 2000,
      expiresAtMs: 10000,
      bodyBytes: 6,
      content: 'Answer',
    },
  });
  await vi.advanceTimersByTimeAsync(CONVERSATION_POLL_MS);
  expect(state.getSnapshot().details[requestId]?.final).toMatchObject({ content: 'Answer' });
  expect(port.show).toHaveBeenCalledTimes(2);
  state.setActive(false);
  await vi.advanceTimersByTimeAsync(CONVERSATION_POLL_MS * 10);
  expect(port.list).toHaveBeenCalledTimes(5);
  state.setActive(true);
  await vi.advanceTimersByTimeAsync(0);
  expect(port.show).toHaveBeenCalledTimes(3);
  state.dispose();
  expect(vi.getTimerCount()).toBe(0);
});

it('releases bodies displaced from the bounded chat window', async () => {
  const { state, port } = fixture();
  state.setActive(true);
  await vi.advanceTimersByTimeAsync(0);
  const newerId = 'req_33333333-3333-4333-8333-333333333333';
  port.list.mockResolvedValue({
    items: [{ ...summary, requestId: newerId, preview: 'Newer request' }],
    nextBefore: null,
  });
  port.show.mockResolvedValue({ ...detail, requestId: newerId });
  await vi.advanceTimersByTimeAsync(CONVERSATION_POLL_MS);
  expect(state.getSnapshot().details[requestId]).toBeUndefined();
  expect(Object.keys(state.getSnapshot().details)).toEqual([newerId]);
  expect(port.show).toHaveBeenLastCalledWith(newerId, expect.any(AbortSignal));
  state.dispose();
});

it('limits body fetch concurrency and fences all unfinished reads when closed', async () => {
  const ids = [
    requestId,
    'req_33333333-3333-4333-8333-333333333333',
    'req_44444444-4444-4444-8444-444444444444',
  ];
  const pending = new Map<string, { resolve(detail: HistoryDetail): void; signal?: AbortSignal }>();
  const port: RequestHistoryPort = {
    list: vi.fn(async () => ({
      items: ids.map((id) => ({ ...summary, requestId: id })),
      nextBefore: null,
    })),
    show: vi.fn(
      (id, signal) =>
        new Promise<HistoryDetail>((resolve) => {
          pending.set(id, { resolve, signal });
        })
    ),
    receipt: vi.fn(),
  };
  const state = createConversationState(port, { recipientId });
  state.setActive(true);
  await vi.advanceTimersByTimeAsync(0);
  expect(port.show).toHaveBeenCalledTimes(2);
  pending.get(ids[0]!)!.resolve(detail);
  await vi.advanceTimersByTimeAsync(0);
  expect(port.show).toHaveBeenCalledTimes(3);
  state.setActive(false);
  for (const id of ids.slice(1)) {
    expect(pending.get(id)!.signal?.aborted).toBe(true);
    pending.get(id)!.resolve({ ...detail, requestId: id });
  }
  await vi.advanceTimersByTimeAsync(0);
  expect(Object.keys(state.getSnapshot().details)).toEqual([requestId]);
  expect(vi.getTimerCount()).toBe(0);
  state.dispose();
});

it('aborts and fences in-flight reads when closed, then obtains fresh state on reopening', async () => {
  let resolve!: (page: HistoryPage) => void;
  let signal: AbortSignal | undefined;
  const port: RequestHistoryPort = {
    list: vi.fn((_query, lifetime) => {
      signal = lifetime;
      return new Promise<HistoryPage>((done) => {
        resolve = done;
      });
    }),
    show: vi.fn(),
    receipt: vi.fn(),
  };
  const state = createConversationState(port, { recipientId });
  state.setActive(true);
  state.setActive(false);
  expect(signal?.aborted).toBe(true);
  resolve({ items: [summary], nextBefore: null });
  await vi.advanceTimersByTimeAsync(0);
  expect(state.getSnapshot().page).toBeUndefined();
  expect(port.show).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
  state.dispose();
});

it('stops on observation deadline without changing request completion and resumes explicitly', async () => {
  const { state, port } = fixture();
  state.setActive(true);
  await vi.advanceTimersByTimeAsync(CONVERSATION_OBSERVATION_MS);
  expect(state.getSnapshot()).toMatchObject({
    paused: true,
    details: { [requestId]: { final: { status: 'not_submitted' } } },
  });
  const count = port.list.mock.calls.length;
  await vi.advanceTimersByTimeAsync(CONVERSATION_POLL_MS * 5);
  expect(port.list).toHaveBeenCalledTimes(count);
  state.refresh();
  await vi.advanceTimersByTimeAsync(0);
  expect(state.getSnapshot().paused).toBe(false);
  expect(port.list).toHaveBeenCalledTimes(count + 1);
  state.dispose();
});

it('stops retrying service failures automatically and keeps earlier results', async () => {
  const { state, port } = fixture();
  state.setActive(true);
  await vi.advanceTimersByTimeAsync(0);
  port.list.mockRejectedValue(new Error('Disconnected'));
  await vi.advanceTimersByTimeAsync(CONVERSATION_POLL_MS * 4);
  expect(port.list).toHaveBeenCalledTimes(2);
  expect(state.getSnapshot()).toMatchObject({
    details: { [requestId]: detail },
    error: expect.stringContaining('refresh to retry'),
  });
  state.dispose();
});

it('uses older-page cursors and does not expose a detail returned from another scope', async () => {
  const { state, port } = fixture();
  const before = { preparedAtMs: summary.preparedAtMs, requestId };
  port.list.mockResolvedValueOnce({ items: [summary], nextBefore: before });
  port.show.mockResolvedValue({ ...detail, recipientId: '33333333-3333-4333-8333-333333333333' });
  state.setActive(true);
  await vi.advanceTimersByTimeAsync(0);
  expect(state.getSnapshot().details[requestId]).toBeUndefined();
  expect(state.getSnapshot().detailErrors[requestId]).toBeDefined();
  state.older();
  await vi.advanceTimersByTimeAsync(0);
  expect(port.list).toHaveBeenLastCalledWith(
    { recipientId, before, limit: CONVERSATION_PAGE_SIZE },
    expect.any(AbortSignal)
  );
  state.latest();
  await vi.advanceTimersByTimeAsync(0);
  expect(port.list).toHaveBeenLastCalledWith(
    { recipientId, limit: CONVERSATION_PAGE_SIZE },
    expect.any(AbortSignal)
  );
  state.dispose();
});
