import { mapGeometry } from '../world-map/map-source.js';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import maps from '../../../../../../../contracts/office/map-v1-vectors.json';
import { builtinFurniture } from '../blocks/block-contract.js';
import { decodeWorldDocument } from '../world-map/world-contract.js';
import {
  decodeWorldSnapshot,
  decodeWorldWrite,
  WorldValidationError,
  WorldConflict,
  WORLD_ENVELOPE_BYTES,
} from '../world-map/world-port.js';
import { LocalHttpError, startLocalRuntime } from './local-runtime.js';

const worldId = '10000000-0000-4000-8000-000000000001';
const objectId = '20000000-0000-4000-8000-000000000001';
const layout = decodeWorldDocument({
  version: 1,
  map: maps.lobby,
  objects: [
    {
      id: objectId,
      kind: 'decoration',
      placement: builtinFurniture('chair', 0, 0, 0),
      surface: { type: 'floor' },
      extension: {
        definition: 'tmt-whiteboard',
        binding: { kind: 'whiteboard', documentId: 'lobby' },
      },
    },
  ],
});
const basis = 'a'.repeat(64);
const preview = {
  worldId: null,
  revision: 0,
  legacyBasis: basis,
  layout,
  updatedAtMs: 0,
  changed: false,
};
const saved = { worldId, revision: 1, legacyBasis: null, layout, updatedAtMs: 100, changed: true };
const write = { expectedRevision: 0, legacyBasis: basis, layout };
beforeEach(() => history.replaceState(null, '', `/local#token=${'a'.repeat(43)}`));
afterEach(() => vi.unstubAllGlobals());

it('cancels all initial directory and artwork requests through the same mounted lifetime', async () => {
  const signals: AbortSignal[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (_url: string, init?: RequestInit) => {
      const signal = init!.signal!;
      signals.push(signal);
      return new Promise<Response>((_resolve, reject) =>
        signal.addEventListener('abort', () => reject(new DOMException('Aborted', 'AbortError')), {
          once: true,
        })
      );
    })
  );
  const runtime = startLocalRuntime(window.location),
    lifetime = new AbortController();
  const reads = [
    runtime.profiles.list(lifetime.signal),
    runtime.avatars.list(lifetime.signal),
    runtime.resolveProps(
      [{ ...builtinFurniture('desk', 0, 0, 0), prop: `sha256:${'f'.repeat(64)}/custom` }],
      lifetime.signal
    ),
  ];
  expect(signals).toHaveLength(3);
  lifetime.abort();
  expect((await Promise.allSettled(reads)).every((result) => result.status === 'rejected')).toBe(
    true
  );
  expect(signals.every((signal) => signal.aborted)).toBe(true);
  runtime.dispose();
});

it('validates snapshots and migration fences without accepting client-authored authority', () => {
  expect(decodeWorldSnapshot(preview)).toEqual(preview);
  expect(decodeWorldSnapshot(saved)).toEqual(saved);
  expect(decodeWorldWrite(write)).toEqual(write);
  for (const patch of [
    { legacyBasis: null },
    { changed: true },
    { updatedAtMs: 1 },
    { revision: -1 },
    { extra: true },
  ])
    expect(() => decodeWorldSnapshot({ ...preview, ...patch })).toThrow();
  for (const patch of [{ worldId: null }, { legacyBasis: basis }, { updatedAtMs: 0 }])
    expect(() => decodeWorldSnapshot({ ...saved, ...patch })).toThrow();
  for (const patch of [{ legacyBasis: null }, { expectedRevision: 1 }, { actor: 'owner' }])
    expect(() => decodeWorldWrite({ ...write, ...patch })).toThrow();
});

it('uses shared authenticated bounded transport and accepts only the matching canonical result', async () => {
  const fetch = vi.fn(
    async (_url: string, init?: RequestInit) =>
      new Response(JSON.stringify(init?.method === 'PUT' ? saved : preview))
  );
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  await expect(runtime.world.show()).resolves.toEqual(preview);
  // Native normalization coalesces subdivided brush runs. It does not reorder objects.
  const split = {
    ...write,
    layout: {
      ...layout,
      map: {
        ...mapGeometry(layout.map),
        floor: [
          { y: 0, start: 0, end: 1, areaId: layout.map.primaryLobbyId },
          { y: 0, start: 1, end: 3, areaId: layout.map.primaryLobbyId },
          ...mapGeometry(layout.map).floor.slice(1),
        ],
      },
    },
  };
  await expect(runtime.world.save(split)).resolves.toEqual(saved);
  expect(fetch).toHaveBeenLastCalledWith(
    '/api/v1/local/world',
    expect.objectContaining({
      method: 'PUT',
      body: JSON.stringify(split),
      headers: { Authorization: `Bearer ${'a'.repeat(43)}`, 'Content-Type': 'application/json' },
    })
  );
  for (const invalid of [
    { ...saved, revision: 2 },
    { ...saved, changed: false },
    { ...saved, layout: { ...layout, objects: [] } },
  ]) {
    fetch.mockImplementation(async () => new Response(JSON.stringify(invalid)));
    await expect(runtime.world.save(write)).rejects.toThrow('Unexpected Office world save result');
  }
  runtime.dispose();
});

it('retains affected object diagnostics and never retries or acknowledges a failed save', async () => {
  const issues = [{ objectId, reason: 'outsideFloor' }];
  const fetch = vi.fn(
    async () =>
      new Response(
        JSON.stringify({ error: 'WORLD_INVALID', message: 'Resolve affected objects.', issues }),
        { status: 400 }
      )
  );
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  await expect(runtime.world.save(write)).rejects.toEqual(
    new WorldValidationError('Resolve affected objects.', issues)
  );
  expect(fetch).toHaveBeenCalledTimes(1);
  fetch.mockImplementation(
    async () => new Response('{"error":"WORLD_REVISION_CONFLICT"}', { status: 409 })
  );
  await expect(runtime.world.save(write)).rejects.toBeInstanceOf(WorldConflict);
  expect(fetch).toHaveBeenCalledTimes(2);
  fetch.mockImplementation(async () => new Response('{"error":"OTHER_CONFLICT"}', { status: 409 }));
  await expect(runtime.world.save(write)).rejects.toMatchObject({
    status: 409,
    code: 'OTHER_CONFLICT',
  } satisfies Partial<LocalHttpError>);
  expect(fetch).toHaveBeenCalledTimes(3);
  expect(write.layout.objects[0]!.extension?.binding).toEqual({
    kind: 'whiteboard',
    documentId: 'lobby',
  });
  runtime.dispose();
});

it('bounds responses and forwards cancellation through the existing runtime owner', async () => {
  const fetch = vi.fn(
    async (_url: string, _init?: RequestInit) =>
      new Response('{}', { headers: { 'Content-Length': String(WORLD_ENVELOPE_BYTES + 1) } })
  );
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  await expect(runtime.world.show()).rejects.toThrow();
  const controller = new AbortController();
  controller.abort();
  fetch.mockImplementation(async (_url, init) => {
    expect(init?.signal?.aborted).toBe(true);
    throw new DOMException('Aborted', 'AbortError');
  });
  await expect(runtime.world.show(controller.signal)).rejects.toMatchObject({ name: 'AbortError' });
  runtime.dispose();
});
