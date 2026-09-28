import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { startLocalRuntime } from './local-runtime.js';
import { decodeRoom, decodeRoomList, decodeRoomWrite } from './room-contract.js';
import { RoomRosterChanged } from './dispatch-contract.js';

const id = '11111111-1111-4111-8111-111111111111';
const alice = '22222222-2222-4222-8222-222222222222';
const room = { id, name: 'Design room', revision: 1, retired: false, memberIds: [alice] };
const input = { expectedRevision: 0, name: room.name, memberIds: room.memberIds };
beforeEach(() => history.replaceState(null, '', `/local#token=${'a'.repeat(43)}`));
afterEach(() => vi.unstubAllGlobals());

it('normalizes bounded members but rejects malformed room projections and writes', () => {
  expect(decodeRoomWrite({ ...input, memberIds: [alice, alice] })).toEqual(input);
  expect(decodeRoom(room)).toEqual(room);
  expect(decodeRoomList([])).toEqual([]);
  expect(() => decodeRoomList([room, room])).toThrow('Duplicate room');
  for (const patch of [
    { revision: 0 },
    { memberIds: [alice, alice] },
    { memberIds: ['everyone'] },
    { name: '\n' },
    { name: 'x'.repeat(81) },
    { isOnline: true },
    { retired: 'false' },
  ]) {
    expect(() => decodeRoom({ ...room, ...patch })).toThrow();
  }
  for (const patch of [
    { expectedRevision: -1 },
    { expectedRevision: Number.MAX_SAFE_INTEGER },
    { name: '\uD800' },
    { memberIds: Array(65).fill(alice) },
  ]) {
    expect(() => decodeRoomWrite({ ...input, ...patch })).toThrow();
  }
});

it('retires only the explicit revision and verifies the retained retirement receipt', async () => {
  const retired = { ...room, revision: 2, retired: true };
  const fetch = vi.fn(async () => new Response(JSON.stringify(retired)));
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  await expect(runtime.rooms.retire(id, 1)).resolves.toEqual(retired);
  expect(fetch).toHaveBeenCalledWith(
    `/api/v1/local/rooms/${id}/retire`,
    expect.objectContaining({
      method: 'POST',
      body: JSON.stringify({ expectedRevision: 1 }),
    })
  );
  for (const invalid of [
    { ...retired, retired: false },
    { ...retired, revision: 3 },
    { ...retired, id: alice },
  ]) {
    fetch.mockResolvedValueOnce(new Response(JSON.stringify(invalid)));
    await expect(runtime.rooms.retire(id, 1)).rejects.toThrow('Unexpected room retirement receipt');
  }
  expect(() => decodeRoomList([retired])).toThrow('Retired room in active list');
  runtime.dispose();
});

it('lists and conditionally saves through shared auth, retaining the expected revision and checking the receipt', async () => {
  const fetch = vi.fn(
    async (_url: string, init?: RequestInit) =>
      new Response(JSON.stringify(init?.method === 'PUT' ? room : [room]))
  );
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  await expect(runtime.rooms.list()).resolves.toEqual([room]);
  await expect(runtime.rooms.save(id, input)).resolves.toEqual(room);
  expect(fetch).toHaveBeenNthCalledWith(
    2,
    `/api/v1/local/rooms/${id}`,
    expect.objectContaining({
      method: 'PUT',
      body: JSON.stringify(input),
      headers: { Authorization: `Bearer ${'a'.repeat(43)}`, 'Content-Type': 'application/json' },
    })
  );
  await expect(runtime.rooms.save('../bad', input)).rejects.toThrow();
  expect(fetch).toHaveBeenCalledTimes(2);
  fetch.mockImplementation(async () => new Response(JSON.stringify({ ...room, memberIds: [] })));
  await expect(runtime.rooms.save(id, input)).rejects.toThrow('Unexpected room receipt');
  runtime.dispose();
});

it('keeps room preview changes distinct from uncertain dispatch outcomes', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => new Response('{"error":"ROOM_ROSTER_CHANGED"}', { status: 409 }))
  );
  const runtime = startLocalRuntime(window.location);
  await expect(
    runtime.dispatch.send({
      operationId: id,
      recipientIds: [alice],
      message: 'Review',
      room: { kind: 'roster', roomId: id, revision: 1 },
    })
  ).rejects.toBeInstanceOf(RoomRosterChanged);
  runtime.dispose();
});
