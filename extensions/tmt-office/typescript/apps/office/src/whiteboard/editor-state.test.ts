import { describe, expect, it, vi } from 'vitest';
import example from '../../../../../../../contracts/office/whiteboard-scene-v1.json';
import { decodeWhiteboardScene } from './scene-contract.js';
import { currentScene } from './history.js';
import { createWhiteboardState } from './editor-state.js';
import type { WhiteboardDocument, WhiteboardPort, WhiteboardReceipt } from './document-contract.js';

const document = (): WhiteboardDocument => ({
  id: 'lobby',
  revision: 1,
  updatedAtMs: 100,
  scene: decodeWhiteboardScene(example),
});
const receipt = (operationId: string): WhiteboardReceipt => ({
  documentId: 'lobby',
  revision: 2,
  updatedAtMs: 200,
  changed: true,
  operationId,
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function port(): WhiteboardPort {
  return {
    show: vi.fn(async () => document()),
    save: vi.fn(async (_id, input) => receipt(input.operationId)),
  };
}

describe('mounted whiteboard state', () => {
  it('loads without writing, edits locally and saves one owned revision intent', async () => {
    const api = port();
    const state = createWhiteboardState(api, 'lobby', () => 'operation');
    await state.load();
    expect(api.save).not.toHaveBeenCalled();
    expect(state.getSnapshot()).toMatchObject({ busy: false, dirty: false });
    state.edit({ ...document().scene, elements: [] });
    expect(state.getSnapshot().dirty).toBe(true);
    state.undo();
    expect(state.getSnapshot().dirty).toBe(false);
    state.redo();
    expect(state.getSnapshot().dirty).toBe(true);
    await state.save();
    expect(api.save).toHaveBeenCalledExactlyOnceWith(
      'lobby',
      {
        expectedRevision: 1,
        operationId: 'operation',
        scene: { ...document().scene, elements: [] },
      },
      expect.any(AbortSignal)
    );
    expect(state.getSnapshot()).toMatchObject({ dirty: false, document: { revision: 2 } });
    await state.save();
    expect(api.save).toHaveBeenCalledTimes(1);
    state.dispose();
  });

  it('freezes an unconfirmed intent and retries exactly, preserving edits and undo history', async () => {
    const api = port();
    const pending = deferred<WhiteboardReceipt>();
    vi.mocked(api.save).mockReturnValueOnce(pending.promise);
    const id = vi.fn(() => 'operation');
    const state = createWhiteboardState(api, 'lobby', id);
    await state.load();
    state.edit({ ...document().scene, background: '#ffffff' });
    const history = state.getSnapshot().history;
    const saving = state.save();
    state.edit({ ...document().scene, elements: [] });
    state.undo();
    expect(state.getSnapshot().history).toBe(history);
    pending.reject(new Error('response lost after commit'));
    await saving;
    expect(state.getSnapshot()).toMatchObject({ dirty: true, unconfirmed: true, busy: false });
    state.redo();
    state.edit(document().scene);
    expect(state.getSnapshot().history).toBe(history);
    await state.save();
    expect(api.save).toHaveBeenCalledTimes(2);
    expect(vi.mocked(api.save).mock.calls[1]?.[1]).toBe(vi.mocked(api.save).mock.calls[0]?.[1]);
    expect(id).toHaveBeenCalledTimes(1);
    expect(state.getSnapshot()).toMatchObject({
      dirty: false,
      unconfirmed: false,
      document: { revision: 2 },
    });
    state.undo();
    expect(state.getSnapshot().dirty).toBe(true);
    state.dispose();
  });

  it('only replaces a draft after an explicit successful reload, and fences completion after disposal', async () => {
    const api = port();
    const state = createWhiteboardState(api, 'lobby');
    await state.load();
    state.edit({ ...document().scene, elements: [] });
    vi.mocked(api.show).mockRejectedValueOnce(new Error('offline'));
    await state.load();
    expect(currentScene(state.getSnapshot().history!).elements).toEqual([]);
    expect(state.getSnapshot().dirty).toBe(true);
    await state.load();
    expect(currentScene(state.getSnapshot().history!)).toEqual(example);
    const waiting = deferred<WhiteboardDocument>();
    vi.mocked(api.show).mockReturnValueOnce(waiting.promise);
    const changed = vi.fn();
    state.subscribe(changed);
    const loading = state.load();
    const before = state.getSnapshot();
    state.dispose();
    changed.mockClear();
    waiting.resolve({ ...document(), revision: 8 });
    await loading;
    expect(changed).not.toHaveBeenCalled();
    expect(state.getSnapshot()).toBe(before);
    expect(vi.mocked(api.show).mock.calls.at(-1)?.[1]?.aborted).toBe(true);
  });
});
