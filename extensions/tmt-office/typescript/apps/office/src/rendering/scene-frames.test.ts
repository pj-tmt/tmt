import { afterEach, expect, it, vi } from 'vitest';
import { createSceneFrames } from './scene-frames.js';

afterEach(() => vi.restoreAllMocks());

function harness() {
  let sequence = 0;
  const pending = new Map<number, FrameRequestCallback>();
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
    pending.set(++sequence, callback);
    return sequence;
  });
  vi.spyOn(window, 'cancelAnimationFrame').mockImplementation((id) => {
    pending.delete(id);
  });
  const visibility = vi.spyOn(document, 'hidden', 'get').mockReturnValue(false);
  const render = vi.fn();
  const frames = createSceneFrames(render);
  function flush() {
    const callbacks = [...pending.values()];
    pending.clear();
    callbacks.forEach((callback) => callback(0));
  }
  function hide(hidden: boolean) {
    visibility.mockReturnValue(hidden);
    document.dispatchEvent(new Event('visibilitychange'));
  }
  return { pending, render, frames, flush, hide };
}

it('coalesces invalidations into one frame and stays idle afterward', () => {
  const scene = harness();
  try {
    expect(scene.pending.size).toBe(0);
    for (let index = 0; index < 100; index++) scene.frames.invalidate();
    expect(scene.pending.size).toBe(1);
    scene.flush();
    expect(scene.render).toHaveBeenCalledTimes(1);
    expect(scene.pending.size).toBe(0);
    scene.flush();
    expect(scene.render).toHaveBeenCalledTimes(1);
  } finally {
    scene.frames.dispose();
  }
});

it('cancels hidden work and renders the latest dirty scene once on return', () => {
  const scene = harness();
  try {
    scene.frames.invalidate();
    scene.hide(true);
    expect(scene.pending.size).toBe(0);
    scene.frames.invalidate();
    scene.frames.invalidate();
    scene.flush();
    expect(scene.render).not.toHaveBeenCalled();
    scene.hide(false);
    expect(scene.pending.size).toBe(1);
    scene.flush();
    expect(scene.render).toHaveBeenCalledTimes(1);
    scene.hide(true);
    scene.hide(false);
    expect(scene.pending.size).toBe(0);
  } finally {
    scene.frames.dispose();
  }
});

it('removes queued work and cannot be revived after disposal', () => {
  const scene = harness();
  scene.frames.invalidate();
  scene.frames.dispose();
  scene.frames.dispose();
  scene.frames.invalidate();
  scene.hide(true);
  scene.hide(false);
  scene.flush();
  expect(scene.pending.size).toBe(0);
  expect(scene.render).not.toHaveBeenCalled();
});
