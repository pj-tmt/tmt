import { afterEach, expect, it, vi } from 'vite-plus/test';
import type { Connection } from '../src/connection.js';
import { Writer, memoryRecords } from '../src/writer.js';

// The serve's owner fold can already hold a checkpoint of this device's stream at its head. This
// device's own checkpoint there is then refused with `CONFLICT`: it was never stored, so it must
// not stay in the saved writer state to be replayed (and refused again) before every later write,
// and the combine must not report success (#4275).
vi.mock('@tmt/colab-client', async (original) => ({
  ...(await original<typeof import('@tmt/colab-client')>()),
  Envelope: {
    seal: async () => ({ hash: async () => new Uint8Array(32), toJson: () => ({}) }),
  },
}));
vi.mock('../src/own-seal.js', () => ({
  sealOwnUpdate: async (_a: unknown, head: { seq: bigint }, update: Uint8Array) => ({
    seq: String(head.seq + 1n),
    envelopeHash: 'hash',
    envelope: 'envelope',
    update: update.slice(),
  }),
  sealedRevision: () => 1,
}));
const device = '00000000-0000-4000-8000-000000000004';
const stream = 'own-stream';

class Channel {
  onmessage: ((event: { data: unknown }) => void) | null = null;
  postMessage() {}
  close() {}
}
afterEach(() => vi.unstubAllGlobals());

function leader() {
  vi.stubGlobal('BroadcastChannel', Channel);
  vi.stubGlobal('navigator', {
    locks: { request: (_key: string, _options: unknown, fn: () => unknown) => fn() },
  });
  const appended: { seq: string }[] = [];
  let conflict = true;
  const connection = {
    ready: Promise.resolve(),
    active: true,
    admission: {
      registration: { deviceId: device, keys: { sign: new Uint8Array(32) } },
      root: new Uint8Array(32),
      head: { revision: 1n },
      space: 's',
      page: 'p',
      epoch: '1',
      ownAuthor: async () => {},
    },
    objects: {
      head: () => ({ seq: 12n, hash: new Uint8Array(32) }),
      tail: () => 12,
    },
    run: (fn: () => Promise<unknown>) => fn(),
    fold: {
      run: async (command: { type: string }) => ({
        // `own-conflict` answers with no held update; `encode-own` and `check` answer as the
        // worker would for this stream.
        update: new Uint8Array(command.type === 'encode-own' ? [1] : []),
      }),
    },
    waitForOwnRecords: async () => {},
    append: async (entry: { seq: string }) => {
      appended.push(entry);
      // The serve answers the checkpoint at the folded head with CONFLICT; later appends land.
      if (conflict && appended.length === 1) throw new Error('CONFLICT');
      conflict = false;
    },
  };
  const records = memoryRecords();
  const writer = new Writer(stream, async () => connection as unknown as Connection, records);
  return { writer, records, appended };
}

it('drops a checkpoint the serve refused with CONFLICT, reports it not combined, and seals the next write', async () => {
  const { writer, records, appended } = leader();
  try {
    await Promise.resolve();
    // The combine does not succeed silently, and does not throw into the caller.
    await expect(writer.compact()).resolves.toEqual({ outcome: 'not-combined' });
    // Nothing is left to replay before a later write.
    const saved = await records<{ pending: unknown }>(stream);
    expect(saved?.pending).toBeNull();
    // The next write is sealed on its own and lands once; the refused checkpoint is not resent.
    await writer.submit(new Uint8Array([9]));
    expect(appended).toHaveLength(2);
    expect(appended[1]!.seq).toBe('13');
  } finally {
    writer.close();
  }
});
