import { afterEach, expect, it, vi } from 'vite-plus/test';
import type { Connection } from '../src/connection.js';
import type { OwnRecord } from '../src/fold-protocol.js';
import { Writer } from '../src/writer.js';

class Relay {
  static current: Relay;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  sent: unknown[] = [];
  constructor() {
    Relay.current = this;
  }
  postMessage(value: { type: string; id: string }) {
    this.sent.push(value);
    // The leader's append receipt arrives before this follower's own broadcast.
    queueMicrotask(() => this.onmessage?.({ data: { type: 'result', id: value.id, ok: true } }));
  }
  close() {}
}
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('keeps a follower own publication pending after relay success until local admission', async () => {
  vi.useFakeTimers();
  vi.stubGlobal('BroadcastChannel', Relay);
  vi.stubGlobal('navigator', { locks: { request: () => new Promise(() => {}) } });
  let admit!: () => void;
  const observed = new Promise<void>((resolve) => {
    admit = resolve;
  });
  const records: OwnRecord[] = [{ root: 'messages', key: 'comment', value: { text: 'Hello' } }];
  const update = new Uint8Array([1]);
  const connection = {
    ready: Promise.resolve(),
    admission: { registration: { deviceId: 'owner' } },
    run: (fn: () => Promise<unknown>) => fn(),
    fold: { run: vi.fn(async () => ({ update })) },
    waitForOwnRecords: vi.fn(() => observed),
    append: vi.fn(),
  };
  const writer = new Writer('follower', async () => connection as unknown as Connection);
  let completed = false;
  const publication = writer.submitOwnRecords(records).then(() => {
    completed = true;
  });
  try {
    await vi.advanceTimersByTimeAsync(0);
    expect(Relay.current.sent).toHaveLength(1);
    expect(completed).toBe(false);
    expect(connection.waitForOwnRecords).toHaveBeenCalledWith(records);
    admit();
    await publication;
    expect(completed).toBe(true);
    expect(connection.append).not.toHaveBeenCalled();
    expect(Relay.current.sent).toHaveLength(1);
    expect(update).toEqual(new Uint8Array([0]));
  } finally {
    writer.close();
  }
});
