import { expect, it, vi } from 'vite-plus/test';
import scene from '../../../../../contracts/whiteboard-scene-v1.json';
import { decodeWhiteboardDocument } from './document-contract.js';
import { decodeWhiteboardSnapshot } from './snapshot-contract.js';
import type { WhiteboardSnapshot, WhiteboardSnapshotPort } from './snapshot-contract.js';
import { createSnapshotState } from './snapshot-state.js';

const id = '11111111-1111-4111-8111-111111111111';
const document = () =>
  decodeWhiteboardDocument({ id: 'lobby', revision: 1, scene, updatedAtMs: 100 });
const rendered = new Blob(['rendered pixels'], { type: 'image/png' });
const stored = new Blob(['normalized pixels'], { type: 'image/png' });
function port(): WhiteboardSnapshotPort {
  return {
    capture: vi.fn(async (documentId, input) =>
      decodeWhiteboardSnapshot({
        id: input.operationId,
        documentId,
        documentRevision: input.expectedRevision,
        scene,
        selectedElementIds: input.selectedElementIds,
        annotation: input.annotation,
        createdAtMs: 200,
      })
    ),
    attachImage: vi.fn(async () => stored),
    show: vi.fn(),
    image: vi.fn(),
  };
}

it('renders the retained capture and exposes the actual stored image without saving or dispatching', async () => {
  const api = port();
  const render = vi.fn(async () => rendered);
  const owner = createSnapshotState(api, render, () => id);
  const source = document();
  const ids = [source.scene.elements[0]!.id];
  await owner.capture(source, ids, 'Check the highlighted note.');
  const captured = owner.getSnapshot().captured!;
  expect(render).toHaveBeenCalledExactlyOnceWith(captured);
  expect(api.attachImage).toHaveBeenCalledExactlyOnceWith(id, rendered, expect.any(AbortSignal));
  expect(owner.getSnapshot()).toMatchObject({ busy: false, pending: false, image: stored });
  source.scene.elements.length = 0;
  ids.length = 0;
  expect(captured.scene.elements.length).toBeGreaterThan(0);
  expect(captured.selectedElementIds).toHaveLength(1);
  await owner.capture(document(), [], 'different intent');
  expect(api.capture).toHaveBeenCalledTimes(1);
  owner.reset();
  expect(owner.getSnapshot()).toEqual({
    busy: false,
    pending: false,
    captured: undefined,
    intent: undefined,
    image: undefined,
    error: undefined,
  });
  owner.dispose();
});

it('retries uncertain capture with one immutable intent and image failure with the same PNG', async () => {
  const api = port();
  vi.mocked(api.capture).mockRejectedValueOnce(new Error('capture response lost'));
  vi.mocked(api.attachImage).mockRejectedValueOnce(new Error('image response lost'));
  const render = vi.fn(async () => rendered);
  const operation = vi.fn(() => id);
  const owner = createSnapshotState(api, render, operation);
  await owner.capture(document(), [], 'Original annotation');
  expect(owner.getSnapshot()).toMatchObject({ pending: true, busy: false });
  expect(render).not.toHaveBeenCalled();
  await owner.capture(document(), [], 'Do not replace');
  expect(owner.getSnapshot().intent).toMatchObject({
    operationId: id,
    expectedRevision: 1,
    selectedElementIds: [],
    annotation: 'Original annotation',
  });
  await owner.retry();
  expect(vi.mocked(api.capture).mock.calls[1]![1]).toBe(vi.mocked(api.capture).mock.calls[0]![1]);
  expect(owner.getSnapshot().image).toBeUndefined();
  await owner.retry();
  expect(api.capture).toHaveBeenCalledTimes(2);
  expect(render).toHaveBeenCalledTimes(1);
  expect(operation).toHaveBeenCalledTimes(1);
  expect(vi.mocked(api.attachImage).mock.calls.map((call) => call[1])).toEqual([
    rendered,
    rendered,
  ]);
  expect(owner.getSnapshot()).toMatchObject({ pending: false, image: stored });
  owner.dispose();
});

it('retries failed rendering on the same capture and rejects invalid unsaved input before effects', async () => {
  const api = port();
  const render = vi.fn(async () => rendered).mockRejectedValueOnce(new Error('canvas unavailable'));
  const owner = createSnapshotState(api, render, () => id);
  await owner.capture({ ...document(), revision: 0 }, [], '');
  await owner.capture(document(), ['77777777-7777-4777-8777-777777777777'], '');
  expect(api.capture).not.toHaveBeenCalled();
  await owner.capture(document(), [], 'retry render');
  expect(api.attachImage).not.toHaveBeenCalled();
  await owner.retry();
  expect(api.capture).toHaveBeenCalledTimes(1);
  expect(render).toHaveBeenCalledTimes(2);
  expect(vi.mocked(render).mock.calls[0]).toEqual(vi.mocked(render).mock.calls[1]);
  expect(owner.getSnapshot().image).toBe(stored);
  owner.dispose();
});

it('fences a late capture after disposal and does not render or upload it', async () => {
  const api = port();
  let finish!: (snapshot: WhiteboardSnapshot) => void;
  const waiting = new Promise<WhiteboardSnapshot>((resolve) => {
    finish = resolve;
  });
  vi.mocked(api.capture).mockReturnValueOnce(waiting);
  const render = vi.fn(async () => rendered);
  const owner = createSnapshotState(api, render, () => id);
  const changed = vi.fn();
  owner.subscribe(changed);
  const capturing = owner.capture(document(), [], '');
  owner.reset();
  expect(owner.getSnapshot().busy).toBe(true);
  owner.dispose();
  changed.mockClear();
  finish(
    decodeWhiteboardSnapshot({
      id,
      documentId: 'lobby',
      documentRevision: 1,
      scene,
      selectedElementIds: [],
      annotation: '',
      createdAtMs: 200,
    })
  );
  await capturing;
  expect(changed).not.toHaveBeenCalled();
  expect(render).not.toHaveBeenCalled();
  expect(api.attachImage).not.toHaveBeenCalled();
  expect(vi.mocked(api.capture).mock.calls[0]![2]!.aborted).toBe(true);
});
