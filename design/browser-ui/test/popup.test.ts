import { afterEach, expect, it, vi } from 'vite-plus/test';
import { claimBrowserPopup, placeBrowserPopup } from '../src/popup.js';

it('replaces the previous surface and stale cleanup cannot release its replacement', () => {
  const document = {} as Document;
  const events: string[] = [];
  const first = claimBrowserPopup(document, () => events.push('first'));
  const second = claimBrowserPopup(document, () => events.push('second'));
  first();
  const third = claimBrowserPopup(document, () => events.push('third'));
  expect(events).toEqual(['first', 'second']);
  third();
  second();
  const last = claimBrowserPopup(document, () => events.push('last'));
  expect(events).toEqual(['first', 'second']);
  last();
});

afterEach(() => vi.unstubAllGlobals());
it('defers observer writes and cancels the pending frame on surface cleanup', () => {
  let changed: () => void = () => {};
  const frames = new Map<number, FrameRequestCallback>();
  let next = 0;
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(callback: () => void) {
        changed = callback;
      }
      observe() {}
      disconnect() {}
    },
  );
  const view = {
    innerWidth: 390,
    innerHeight: 800,
    requestAnimationFrame(callback: FrameRequestCallback) {
      frames.set(++next, callback);
      return next;
    },
    cancelAnimationFrame(id: number) {
      frames.delete(id);
    },
    addEventListener() {},
    removeEventListener() {},
  };
  const anchor = {
    ownerDocument: { defaultView: view },
    isConnected: true,
    getClientRects: () => [1],
    getBoundingClientRect: () => ({ left: 8, right: 40, top: 8, bottom: 40 }),
  };
  let reads = 0;
  const surface = {
    style: {},
    scrollHeight: 100,
    offsetHeight: 100,
    clientHeight: 100,
    getBoundingClientRect: () => {
      reads++;
      return { width: 280, height: 100 };
    },
  };
  const dispose = placeBrowserPopup(
    anchor as unknown as HTMLElement,
    surface as unknown as HTMLElement,
    () => {},
  );
  expect(reads).toBe(1);
  changed();
  changed();
  expect(reads).toBe(1);
  expect(frames.size).toBe(1);
  const [id, callback] = [...frames][0];
  frames.delete(id);
  callback(0);
  expect(reads).toBe(2);
  changed();
  expect(frames.size).toBe(1);
  dispose();
  expect(frames.size).toBe(0);
  expect(reads).toBe(2);
});
