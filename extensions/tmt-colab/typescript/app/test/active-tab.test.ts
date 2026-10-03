import { afterEach, expect, it, vi } from 'vite-plus/test';
import { ActiveTab, InactiveTabError } from '../src/active-tab.js';

/** Deterministic API doubles; Chromium also exercises real channels/locks. */
function environment() {
  const channels = new Set<Channel>();
  class Channel {
    onmessage?: (event: { data: unknown }) => void;
    constructor(readonly name: string) {
      channels.add(this);
    }
    postMessage(data: unknown) {
      for (const peer of channels)
        if (peer !== this && peer.name === this.name)
          queueMicrotask(() => {
            if (channels.has(peer)) peer.onmessage?.({ data: structuredClone(data) });
          });
    }
    close() {
      channels.delete(this);
    }
  }
  const tails = new Map<string, Promise<unknown>>();
  const locks = {
    request(name: string, { signal }: { signal: AbortSignal }, action: () => Promise<void>) {
      let reject!: (error: Error) => void;
      const aborted = new Promise<never>((_resolve, no) => {
        reject = no;
      });
      const abort = () => reject(new DOMException('Aborted', 'AbortError'));
      signal.addEventListener('abort', abort, { once: true });
      const previous = tails.get(name) ?? Promise.resolve();
      const next = Promise.race([previous, aborted]).then(async () => {
        signal.removeEventListener('abort', abort);
        if (signal.aborted) throw new DOMException('Aborted', 'AbortError');
        await action();
      });
      tails.set(
        name,
        previous.then(() => next).catch(() => {}),
      );
      return next;
    },
  };
  vi.stubGlobal('BroadcastChannel', Channel);
  vi.stubGlobal('navigator', { locks });
  return { channels, Channel };
}
afterEach(() => vi.unstubAllGlobals());
it('latest takeover deactivates the old tab before Remote starts, and only explicit takeover swaps them', async () => {
  const env = environment();
  const lostA = vi.fn(),
    lostB = vi.fn();
  const a = new ActiveTab('/mount/', lostA),
    b = new ActiveTab('/mount/', lostB);
  const remote = vi.fn(async () => {});
  try {
    await a.takeover();
    await a.run(remote);
    const second = b.takeover();
    expect(b.takeover()).toBe(second);
    await second;
    expect(a.active).toBe(false);
    expect(lostA).toHaveBeenCalledOnce();
    expect(b.active).toBe(true);
    await expect(a.run(remote)).rejects.toBeInstanceOf(InactiveTabError);
    expect(remote).toHaveBeenCalledOnce();
    await b.run(remote);
    await a.takeover();
    expect(a.active).toBe(true);
    expect(b.active).toBe(false);
    expect(lostB).toHaveBeenCalledOnce();
    await expect(b.run(remote)).rejects.toBeInstanceOf(InactiveTabError);
    expect(remote).toHaveBeenCalledTimes(2);
    b.close();
    expect(a.active).toBe(true);
  } finally {
    a.close();
    b.close();
  }
  expect(env.channels.size).toBe(0);
});
it('takeover waits for an already-started call, blocks later calls and cancels a superseded queued tab', async () => {
  const env = environment();
  let lost!: () => void;
  const inactive = new Promise<void>((resolve) => {
    lost = resolve;
  });
  const a = new ActiveTab('/mount/', lost),
    b = new ActiveTab('/mount/', () => {}),
    c = new ActiveTab('/mount/', () => {});
  let finish!: () => void;
  try {
    await a.takeover();
    const running = a.run(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    const pending = b.takeover();
    const cancelled = expect(pending).rejects.toMatchObject({ name: 'AbortError' });
    await inactive;
    expect(a.active).toBe(false);
    expect(b.active).toBe(false);
    const never = vi.fn(async () => {});
    await expect(a.run(never)).rejects.toBeInstanceOf(InactiveTabError);
    expect(never).not.toHaveBeenCalled();
    const latest = c.takeover();
    await cancelled;
    finish();
    await running;
    await latest;
    expect(c.active).toBe(true);
    expect(b.active).toBe(false);
    const stale = new env.Channel('colab-tab:/mount/');
    stale.postMessage({ type: 'takeover', tab: crypto.randomUUID(), at: 0 });
    await Promise.resolve();
    expect(c.active).toBe(true);
    stale.close();
  } finally {
    a.close();
    b.close();
    c.close();
  }
  expect(env.channels.size).toBe(0);
});
