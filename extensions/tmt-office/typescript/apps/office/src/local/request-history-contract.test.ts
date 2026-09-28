import { describe, expect, it } from 'vitest';
import {
  decodeHistoryDetail,
  decodeHistoryPage,
  decodeHistoryQuery,
} from './request-history-contract.js';

const recipientId = '11111111-1111-4111-8111-111111111111';
const roomId = '22222222-2222-4222-8222-222222222222';
const requestId = 'req_33333333-3333-4333-8333-333333333333';
function summary() {
  return {
    requestId,
    roomId: null,
    recipientId,
    sender: { kind: 'unknown', identityId: null },
    kind: 'request',
    preparedAtMs: 1000,
    delivery: 'queued',
    recipientAcknowledged: false,
    final: { status: 'not_submitted' },
    preview: 'Review this',
  };
}
function detail(
  final: unknown = { status: 'not_submitted' },
  prompt: unknown = { status: 'unavailable' }
) {
  const { preview: _preview, ...item } = summary();
  return { ...item, final, prompt };
}

describe('request history boundary', () => {
  it('requires scope, bounds pages and rejects unknown fields', () => {
    expect(decodeHistoryQuery({ recipientId })).toEqual({ recipientId, limit: 20 });
    expect(decodeHistoryQuery({ roomId, limit: 50 })).toEqual({ roomId, limit: 50 });
    expect(decodeHistoryQuery({ roomId, recipientId })).toEqual({ roomId, recipientId, limit: 20 });
    for (const value of [
      {},
      { recipientId: 'Alice' },
      { recipientId, limit: 0 },
      { roomId, limit: 51 },
      { roomId, limit: 1.5 },
      { roomId, actor: 'owner' },
      { roomId, before: { preparedAtMs: 0, requestId } },
    ])
      expect(() => decodeHistoryQuery(value)).toThrow();
  });

  it('validates scope and descending keyset pages without accepting repeated items', () => {
    const first = summary();
    const nextBefore = { preparedAtMs: first.preparedAtMs, requestId };
    expect(
      decodeHistoryPage({ items: [first], nextBefore }, { recipientId, limit: 1 })
    ).toMatchObject({ items: [{ requestId }], nextBefore });
    for (const value of [
      { items: [first, first], nextBefore: null },
      { items: [first], nextBefore: { ...nextBefore, preparedAtMs: 999 } },
      { items: [], nextBefore },
      { items: [{ ...first, recipientId: roomId }], nextBefore: null },
      { items: [{ ...first, preview: 'x'.repeat(161) }], nextBefore: null },
      { items: [{ ...first, proof: 'secret' }], nextBefore: null },
    ])
      expect(() => decodeHistoryPage(value, { recipientId, limit: 1 })).toThrow();
    expect(() =>
      decodeHistoryPage({ items: [first], nextBefore: null }, { recipientId, before: nextBefore })
    ).toThrow('order');
    expect(() => decodeHistoryPage({ items: [first], nextBefore: null }, { roomId })).toThrow(
      'scope'
    );
    expect(
      decodeHistoryPage(
        { items: [{ ...first, preview: 'head\0🤖' }], nextBefore: null },
        { recipientId }
      ).items[0]?.preview
    ).toBe('head\0🤖');
  });

  it('keeps exact detail bodies and nullable pane acknowledgment', () => {
    const message = '  question\n\0🤖';
    const response = '  answer\n';
    const value = {
      ...detail(
        {
          status: 'retained',
          response,
          submittedAtMs: 2000,
          expiresAtMs: 3000,
          bodyBytes: new TextEncoder().encode(response).length,
        },
        {
          status: 'retained',
          message,
          messageBytes: new TextEncoder().encode(message).length,
          expiresAtMs: 3000,
        }
      ),
      delivery: 'sent',
      recipientAcknowledged: null,
    };
    expect(decodeHistoryDetail(value)).toMatchObject({
      final: { content: response },
      prompt: { message },
      recipientAcknowledged: null,
    });
    expect(() =>
      decodeHistoryDetail({ ...value, final: { ...(value.final as object), bodyBytes: 1 } })
    ).toThrow();
    expect(() =>
      decodeHistoryDetail({
        ...value,
        prompt: { status: 'retained', message: '\uD800', messageBytes: 3, expiresAtMs: 3000 },
      })
    ).toThrow();
  });

  it('distinguishes expired, unavailable, not submitted and no reply required', () => {
    for (const final of [
      { status: 'not_submitted' },
      { status: 'expired', submittedAtMs: 2000, expiresAtMs: 3000 },
      { status: 'unavailable', submittedAtMs: 2000, expiresAtMs: 3000 },
    ])
      expect(decodeHistoryDetail(detail(final)).final.status).toBe(final.status);
    expect(
      decodeHistoryDetail({ ...detail({ status: 'not_required' }), kind: 'announcement' }).final
        .status
    ).toBe('not_required');
    expect(() => decodeHistoryDetail(detail({ status: 'not_required' }))).toThrow();
    expect(() =>
      decodeHistoryDetail(detail({ status: 'expired', submittedAtMs: 3000, expiresAtMs: 2000 }))
    ).toThrow();
    expect(() =>
      decodeHistoryDetail(detail({ status: 'not_submitted', response: 'invented' }))
    ).toThrow();
  });
});
