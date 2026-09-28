import { afterEach, expect, it, vi } from 'vitest';
import { createDispatchJournal } from './dispatch-journal.js';
import { createDispatchComposerState } from './dispatch-composer-state.js';
import type { DispatchInput, DispatchReceipt } from './dispatch-contract.js';
import { RoomRosterChanged } from './dispatch-contract.js';

const alice = { id: '11111111-1111-4111-8111-111111111111', name: 'Alice' };
const operationId = '22222222-2222-4222-8222-222222222222';
const intent: DispatchInput = { operationId, recipientIds: [alice.id], message: 'Review this\n' };
const receipt: DispatchReceipt = {
  operationId,
  createdAtMs: 1000,
  items: [{ recipientId: alice.id, requestId: `req_${operationId}`, acceptance: 'queued' }],
};
afterEach(() => sessionStorage.clear());

it('recovers an uncertain send after remount without silently resending', async () => {
  const journal = createDispatchJournal({ kind: 'direct', recipientId: alice.id });
  const send = vi.fn().mockRejectedValue(new Error('Lost response'));
  const lookup = vi.fn(async () => receipt);
  const composition = {
    kind: 'request' as const,
    message: (text: string) => text,
    recipients: [alice],
  };
  const original = createDispatchComposerState({ send }, composition, () => operationId, {
    journal,
    lookup,
  });
  original.change(intent.message, [alice]);
  original.review();
  await original.send();
  expect(journal.read()).toEqual(intent);
  original.dispose();
  const reopened = createDispatchComposerState({ send }, composition, () => operationId, {
    journal,
    lookup,
  });
  expect(reopened.getSnapshot()).toMatchObject({
    review: intent,
    attempted: true,
    recipients: [alice],
  });
  await reopened.recover();
  expect(send).toHaveBeenCalledTimes(1);
  expect(reopened.getSnapshot().receipt).toEqual(receipt);
  expect(journal.read()).toBeUndefined();
  reopened.dispose();
});

it('an absent receipt preserves exact retry intent and a failed journal write prevents new sending', async () => {
  const journal = createDispatchJournal({ kind: 'direct', recipientId: alice.id });
  journal.save(intent);
  const send = vi.fn(async () => receipt);
  const state = createDispatchComposerState(
    { send },
    { kind: 'request', message: (text) => text, recipients: [alice] },
    () => operationId,
    { journal, lookup: vi.fn(async () => null) }
  );
  await state.recover();
  expect(state.getSnapshot().review).toEqual(intent);
  state.change('Changed', []);
  state.edit();
  await state.send();
  expect(send).toHaveBeenCalledExactlyOnceWith(intent, expect.any(AbortSignal));
  state.dispose();
  const failing = createDispatchJournal({ kind: 'direct', recipientId: alice.id }, () => ({
    getItem: () => null,
    removeItem: vi.fn(),
    setItem: () => {
      throw new Error('Quota');
    },
  }));
  const blocked = createDispatchComposerState(
    { send },
    { kind: 'request', message: (text) => text, recipients: [alice] },
    () => operationId,
    { journal: failing, lookup: vi.fn() }
  );
  blocked.change('Question', [alice]);
  blocked.review();
  await blocked.send();
  expect(send).toHaveBeenCalledTimes(1);
  expect(blocked.getSnapshot()).toMatchObject({
    attempted: false,
    error: expect.stringContaining('No new send'),
  });
  blocked.dispose();
});

it('rejects another target, competing intent and malformed recovery rather than silently dropping them', () => {
  const journal = createDispatchJournal({ kind: 'direct', recipientId: alice.id });
  expect(() => journal.save({ ...intent, recipientIds: [operationId] })).toThrow();
  journal.save(intent);
  expect(() => journal.save({ ...intent, message: 'Changed' })).toThrow();
  journal.clear(operationId.replace('2222-', '2223-'));
  expect(journal.read()).toEqual(intent);
  sessionStorage.setItem(`tmt.office.pending-request.${alice.id}`, '{invalid');
  const send = vi.fn();
  const state = createDispatchComposerState(
    { send },
    { kind: 'request', message: (text) => text, recipients: [alice] },
    undefined,
    { journal, lookup: vi.fn() }
  );
  state.change('New', [alice]);
  state.review();
  expect(state.getSnapshot().recoveryBlocked).toBe(true);
  expect(state.getSnapshot().review).toBeUndefined();
  state.discard();
  expect(journal.read()).toBeUndefined();
  expect(state.getSnapshot().recoveryBlocked).toBeUndefined();
  state.dispose();
});

it('isolates pending intents by recipient and room while preserving exact retry scope', async () => {
  const roomA = '33333333-3333-4333-8333-333333333333';
  const roomB = '44444444-4444-4444-8444-444444444444';
  const unscoped = createDispatchJournal({ kind: 'direct', recipientId: alice.id });
  const a = createDispatchJournal({ kind: 'direct', recipientId: alice.id, roomId: roomA });
  const b = createDispatchJournal({ kind: 'direct', recipientId: alice.id, roomId: roomB });
  const scoped = { ...intent, room: { kind: 'direct' as const, roomId: roomA } };
  unscoped.save(intent);
  a.save(scoped);
  expect(b.read()).toBeUndefined();
  expect(() => b.save(scoped)).toThrow();
  expect(() => unscoped.save(scoped)).toThrow();
  expect(() => a.save(intent)).toThrow();
  expect(() =>
    a.save({ ...scoped, room: { kind: 'roster', roomId: roomA, revision: 1 } })
  ).toThrow();
  const send = vi.fn(async () => receipt);
  const state = createDispatchComposerState(
    { send },
    {
      kind: 'request',
      message: (text) => text,
      recipients: [alice],
      context: scoped.room,
    },
    undefined,
    { journal: a, lookup: vi.fn(async () => null) }
  );
  await state.recover();
  expect(() =>
    state.chooseRoom(
      { id: roomB, name: 'Other room', revision: 1, retired: false, memberIds: [alice.id] },
      [alice]
    )
  ).toThrow();
  await state.send();
  expect(send).toHaveBeenCalledExactlyOnceWith(scoped, expect.any(AbortSignal));
  expect(a.read()).toBeUndefined();
  expect(unscoped.read()).toEqual(intent);
  expect(b.read()).toBeUndefined();
  state.dispose();
});

const roomA = '33333333-3333-4333-8333-333333333333';
const roomB = '44444444-4444-4444-8444-444444444444';
const bob = { id: '55555555-5555-4555-8555-555555555555', name: 'Bob' };
const rosterIntent: DispatchInput = {
  ...intent,
  recipientIds: [alice.id, bob.id],
  room: { kind: 'roster', roomId: roomA, revision: 7 },
};

it('keeps a room-wide pending audience separate from direct messages and freezes it across recovery', async () => {
  const journal = createDispatchJournal({ kind: 'roster', roomId: roomA });
  const direct = createDispatchJournal({ kind: 'direct', recipientId: alice.id, roomId: roomA });
  const other = createDispatchJournal({ kind: 'roster', roomId: roomB });
  journal.save(rosterIntent);
  direct.save({ ...intent, room: { kind: 'direct', roomId: roomA } });
  expect(other.read()).toBeUndefined();
  expect(() => other.save(rosterIntent)).toThrow();
  expect(() => journal.save(intent)).toThrow();
  expect(() => journal.save({ ...rosterIntent, kind: 'announcement' })).toThrow();
  const send = vi.fn(async (input: DispatchInput) => ({
    ...receipt,
    items: input.recipientIds.map((recipientId, index) => ({
      recipientId,
      requestId: `req_${index}`,
      acceptance: 'queued' as const,
    })),
  }));
  const state = createDispatchComposerState(
    { send },
    { kind: 'request', message: (text) => text },
    undefined,
    {
      journal,
      lookup: vi.fn(async () => null),
    }
  );
  await state.recover();
  expect(send).not.toHaveBeenCalled();
  state.chooseRoom(
    { id: roomA, name: 'Changed', revision: 8, retired: false, memberIds: [alice.id] },
    [alice]
  );
  state.change('Replacement', [alice]);
  state.edit();
  expect(state.getSnapshot().review).toEqual(rosterIntent);
  await state.send();
  expect(send).toHaveBeenCalledExactlyOnceWith(rosterIntent, expect.any(AbortSignal));
  expect(journal.read()).toBeUndefined();
  expect(direct.read()?.room).toEqual({ kind: 'direct', roomId: roomA });
  state.dispose();
});

it.each([false, true])(
  'handles a rejected stale roster without retaining a competing operation (clear failure: %s)',
  async (clearFails) => {
    const journal = createDispatchJournal({ kind: 'roster', roomId: roomA });
    const clear = vi.spyOn(journal, 'clear');
    if (clearFails)
      clear.mockImplementation(() => {
        throw new Error('Storage unavailable');
      });
    const send = vi.fn().mockRejectedValueOnce(new RoomRosterChanged());
    const nextId = '66666666-6666-4666-8666-666666666666';
    const operation = vi.fn().mockReturnValueOnce(operationId).mockReturnValue(nextId);
    const state = createDispatchComposerState(
      { send },
      { kind: 'request', message: (text) => text },
      operation,
      {
        journal,
        lookup: vi.fn(async () => null),
      }
    );
    state.change(intent.message, []);
    state.chooseRoom(
      { id: roomA, name: 'Design', revision: 7, retired: false, memberIds: [alice.id, bob.id] },
      [alice, bob]
    );
    state.review();
    await state.send();
    expect(clear).toHaveBeenCalledExactlyOnceWith(operationId);
    expect(state.getSnapshot()).toMatchObject({
      roomStale: true,
      recoveryBlocked: clearFails,
      attempted: false,
    });
    state.chooseRoom(
      { id: roomA, name: 'Design', revision: 8, retired: false, memberIds: [bob.id] },
      [bob]
    );
    state.review();
    if (clearFails) {
      expect(state.getSnapshot().review).toBeUndefined();
      expect(journal.read()).toEqual(rosterIntent);
      clear.mockRestore();
      state.discard();
      expect(journal.read()).toBeUndefined();
      expect(state.getSnapshot().recoveryBlocked).toBeUndefined();
    } else {
      expect(journal.read()).toBeUndefined();
      expect(state.getSnapshot().review).toMatchObject({
        operationId: nextId,
        recipientIds: [bob.id],
        room: { kind: 'roster', roomId: roomA, revision: 8 },
      });
      // Saving the re-preview proves the rejected operation no longer blocks the journal.
      journal.save(state.getSnapshot().review!);
      expect(journal.read()?.operationId).toBe(nextId);
    }
    state.dispose();
  }
);
