import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createSceneApplication } from './scene-application.js';

const calls = vi.hoisted(() => ({
  init: vi.fn(),
  destroy: vi.fn(),
  resize: vi.fn(),
  disconnect: vi.fn(),
}));
vi.mock('pixi.js', () => ({
  Application: class {
    canvas = document.createElement('canvas');
    stage = { destroy: vi.fn() };
    screen = { width: 1, height: 1 };
    renderer = { resize: calls.resize };
    init = calls.init;
    render = vi.fn();
    destroy(...args: unknown[]) {
      calls.destroy(...args);
      this.canvas.remove();
    }
  },
}));

beforeEach(() => {
  calls.init.mockResolvedValue(undefined);
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect = calls.disconnect;
    }
  );
});
afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

it('uses a private stopped ticker and disposes the mounted canvas on abort', async () => {
  const host = document.createElement('div');
  const controller = new AbortController();
  const scene = await createSceneApplication(host, controller.signal);
  expect(calls.init).toHaveBeenCalledWith(
    expect.objectContaining({ autoStart: false, sharedTicker: false })
  );
  expect(host.querySelectorAll('canvas')).toHaveLength(1);
  controller.abort();
  scene?.dispose();
  expect(calls.disconnect).toHaveBeenCalledTimes(1);
  expect(calls.destroy).toHaveBeenCalledTimes(1);
  expect(host.querySelectorAll('canvas')).toHaveLength(0);
});

it('cleans up initialization that finishes after unmount without attaching a canvas', async () => {
  let finish!: () => void;
  calls.init.mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      })
  );
  const host = document.createElement('div');
  const controller = new AbortController();
  const pending = createSceneApplication(host, controller.signal);
  controller.abort();
  finish();
  expect(await pending).toBeUndefined();
  expect(calls.destroy).toHaveBeenCalledTimes(1);
  expect(host.children).toHaveLength(0);
});

it('does not initialize an already cancelled mount', async () => {
  const controller = new AbortController();
  controller.abort();
  expect(
    await createSceneApplication(document.createElement('div'), controller.signal)
  ).toBeUndefined();
  expect(calls.init).not.toHaveBeenCalled();
});

it('cleans up and reports an initialization failure', async () => {
  calls.init.mockRejectedValueOnce(new Error('GPU unavailable'));
  await expect(
    createSceneApplication(document.createElement('div'), new AbortController().signal)
  ).rejects.toThrow('GPU unavailable');
  expect(calls.destroy).toHaveBeenCalledTimes(1);
});
