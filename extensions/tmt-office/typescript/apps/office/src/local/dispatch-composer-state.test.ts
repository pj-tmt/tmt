import { describe, expect, it, vi } from 'vitest';
import type { DispatchInput, DispatchPort, DispatchReceipt } from './dispatch-contract.js';
import { DispatchRejected, RoomRosterChanged } from './dispatch-contract.js';
import { createDispatchComposerState } from './dispatch-composer-state.js';

const snapshotId = '11111111-1111-4111-8111-111111111111';
const alice = { id: '22222222-2222-4222-8222-222222222222', name: 'Alice' };
const bob = { id: '33333333-3333-4333-8333-333333333333', name: 'Bob' };
function receipt(input: DispatchInput): DispatchReceipt {
  return {
    operationId: input.operationId,
    createdAtMs: 100,
    items: input.recipientIds.map((recipientId) => ({
      recipientId,
      requestId: `req_${crypto.randomUUID()}`,
      acceptance: 'queued',
    })),
  };
}

describe.each(['request', 'announcement'] as const)('%s composer', (kind) => {
  const createState = (port: DispatchPort) =>
    createDispatchComposerState(port, { kind, message: (text) => text });

  it.each(['invalid', 'operationConflict'] as const)(
    'stops replay after a definite %s rejection without forgetting earlier uncertainty',
    async (reason) => {
      const send = vi
        .fn()
        .mockRejectedValueOnce(new Error('Lost response'))
        .mockRejectedValueOnce(new DispatchRejected(reason));
      const state = createState({ send });
      state.change('Notice', [alice]);
      state.review();
      const review = state.getSnapshot().review;
      await state.send();
      await state.send();
      expect(state.getSnapshot()).toMatchObject({
        rejected: true,
        attempted: true,
        busy: false,
        review,
      });
      expect(state.getSnapshot().error).toContain('does not cancel earlier queued work');
      state.edit();
      await state.send();
      expect(send).toHaveBeenCalledTimes(2);
      expect(state.getSnapshot().review).toBe(review);
      state.discard();
      state.change('New notice', [alice]);
      state.review();
      expect(state.getSnapshot().review?.operationId).not.toBe(review?.operationId);
      expect(state.getSnapshot().rejected).toBeUndefined();
      state.dispose();
    }
  );

  it('freezes the exact preview and names without dispatching while composing, reviewing or editing', async () => {
    const send = vi.fn(async (input: DispatchInput) => receipt(input));
    const state = createState({ send });
    const selected = [{ ...bob }, { ...alice }];
    state.change('  What is missing?\n', selected);
    selected[0]!.name = 'Renamed elsewhere';
    state.review();
    const preview = state.getSnapshot();
    expect(preview.recipients).toEqual([alice, bob]);
    expect(preview.review?.message).toBe('  What is missing?\n');
    expect(preview.review?.kind ?? 'request').toBe(kind);
    state.change('Do not mutate a preview', [bob]);
    expect(state.getSnapshot()).toBe(preview);
    expect(send).not.toHaveBeenCalled();
    state.edit();
    state.change('A deliberate edit', [bob]);
    state.review();
    expect(state.getSnapshot().review?.operationId).not.toBe(preview.review?.operationId);
    expect(send).not.toHaveBeenCalled();
    await state.send();
    expect(send).toHaveBeenCalledExactlyOnceWith(
      state.getSnapshot().review,
      expect.any(AbortSignal)
    );
    await state.send();
    expect(send).toHaveBeenCalledTimes(1);
    state.dispose();
  });

  it('requires a message and recipients before preview or sending', async () => {
    const send = vi.fn();
    const state = createState({ send });
    state.review();
    expect(state.getSnapshot().error).toContain('message');
    state.change('Question', []);
    state.review();
    expect(state.getSnapshot().error).toContain('recipients');
    await state.send();
    expect(send).not.toHaveBeenCalled();
    state.dispose();
  });

  it('retains the same operation through lost responses and disallows editing or parallel sends', async () => {
    let fail!: (error: Error) => void;
    const send = vi
      .fn(async (input: DispatchInput) => receipt(input))
      .mockImplementationOnce(
        () =>
          new Promise((_resolve, reject) => {
            fail = reject;
          })
      );
    const state = createState({ send });
    state.change('Review the note', [alice]);
    state.review();
    const pending = state.send();
    await state.send();
    state.discard();
    state.edit();
    state.change('Changed', [bob]);
    expect(send).toHaveBeenCalledTimes(1);
    fail(new Error('response lost after commit'));
    await pending;
    expect(state.getSnapshot()).toMatchObject({
      attempted: true,
      busy: false,
      text: 'Review the note',
    });
    expect(state.getSnapshot().error).toContain('may already be queued');
    state.edit();
    expect(state.getSnapshot().review).toBeDefined();
    await state.send();
    expect(send.mock.calls[1]![0]).toBe(send.mock.calls[0]![0]);
    expect(state.getSnapshot().receipt?.operationId).toBe(send.mock.calls[0]![0].operationId);
    state.discard();
    expect(state.getSnapshot()).toEqual({
      text: '',
      recipients: [],
      busy: false,
      attempted: false,
    });
    state.dispose();
  });

  it('aborts on disposal and fences a late acceptance without notifying or resending', async () => {
    let finish!: (receipt: DispatchReceipt) => void;
    const send = vi.fn(
      (_input: DispatchInput, _signal?: AbortSignal) =>
        new Promise<DispatchReceipt>((resolve) => {
          finish = resolve;
        })
    );
    const state = createState({ send });
    const changed = vi.fn();
    state.subscribe(changed);
    state.change('Review', [alice]);
    state.review();
    const pending = state.send();
    state.dispose();
    changed.mockClear();
    finish(receipt(send.mock.calls[0]![0]));
    await pending;
    await state.send();
    expect(send.mock.calls[0]![1]?.aborted).toBe(true);
    expect(changed).not.toHaveBeenCalled();
    expect(send).toHaveBeenCalledTimes(1);
  });

  it('requires explicit room re-preview after a roster rejection and freezes the new audience for retry', async () => {
    const send = vi
      .fn(async (input: DispatchInput) => receipt(input))
      .mockRejectedValueOnce(new RoomRosterChanged())
      .mockRejectedValueOnce(new Error('response lost'));
    const state = createState({ send });
    const room = {
      id: snapshotId,
      name: 'Design',
      revision: 1,
      retired: false,
      memberIds: [alice.id],
    };
    state.change('Review the room sketch', []);
    state.chooseRoom(room, [alice]);
    state.review();
    const stale = state.getSnapshot().review!;
    await state.send();
    expect(state.getSnapshot()).toMatchObject({
      attempted: false,
      busy: false,
      roomStale: true,
      review: undefined,
      text: 'Review the room sketch',
    });
    state.review();
    await state.send();
    expect(send).toHaveBeenCalledTimes(1);
    const updated = { ...room, revision: 2, retired: false, memberIds: [alice.id, bob.id] };
    state.chooseRoom(updated, [alice, bob]);
    state.review();
    expect(state.getSnapshot().review?.operationId).not.toBe(stale.operationId);
    updated.memberIds.length = 0;
    await state.send();
    expect(send.mock.calls[1]![0]).toMatchObject({
      room: { kind: 'roster', roomId: snapshotId, revision: 2 },
      recipientIds: [alice.id, bob.id],
    });
    await state.send();
    expect(send.mock.calls[2]![0]).toBe(send.mock.calls[1]![0]);
    state.dispose();
  });
});
