import { afterEach, expect, it, vi } from 'vite-plus/test';
import { encodeBinary } from '@tmt/colab-client';
import type { Admission } from '../src/admission.js';
import { Connection } from '../src/connection.js';
import type { OwnState } from '../src/fold-protocol.js';

const calls = vi.hoisted(() => [] as string[]);
const projection = vi.hoisted(() => ({ own: undefined as OwnState | undefined }));
vi.mock('../src/fold.js', () => ({
  Fold: class {
    async run({ updates }: { updates?: Uint8Array[] }) {
      calls.push('fold');
      return {
        source: updates?.length ? 'updated' : 'initial',
        title: 'Title',
        ...(projection.own ? { own: projection.own } : {}),
      };
    }
    close() {}
  },
}));
vi.mock('../src/catchup.js', () => ({
  Catchup: class {
    updates = [];
    checkpoints = [];
    baseline = null;
    async admitValue() {
      return true;
    }
    close() {}
  },
}));
vi.mock('../src/objects.js', async (original) => ({
  ...(await original<typeof import('../src/objects.js')>()),
  Objects: class {
    ownData = false;
    cursors() {
      return [];
    }
    async admit() {
      calls.push('admit');
      return { namespace: 'content', writer: author, update: new Uint8Array([1]) };
    }
  },
}));
class Socket {
  static OPEN = 1;
  static current: Socket;
  readyState = 1;
  bufferedAmount = 0;
  onopen = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror = null;
  onclose = null;
  send = vi.fn();
  close = vi.fn();
  constructor(readonly url: string | URL) {
    Socket.current = this;
  }
  receive(frame: Record<string, unknown>) {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
}
const page = '10000000-0000-4000-8000-000000000001';
const author = '30000000-0000-4000-8000-000000000001';
const scope = { version: 1, space: 'a'.repeat(32), page, epoch: '1' };
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
async function open(rejectChain = false) {
  calls.length = 0;
  projection.own = undefined;
  vi.stubGlobal('WebSocket', Socket);
  const admission = {
    ...scope,
    root: new Uint8Array(32),
    registration: {
      deviceId: author,
      syncUrl: 'wss://example.test/colab/sync?tmt-session=fixture',
    },
    async chains(value: unknown) {
      calls.push('chain');
      expect(value).toEqual([{ deviceId: author, chain: 'fixture' }]);
      if (rejectChain) throw new Error('Forged chain');
    },
  } as unknown as Admission;
  const publish = vi.fn(),
    failed = vi.fn();
  const connection = new Connection(
    admission,
    new URL('https://example.test/colab/'),
    'private',
    publish,
    failed,
  );
  expect(Socket.current.url).toBe(admission.registration.syncUrl);
  Socket.current.receive({ ...scope, type: 'catchup', streams: [], more: false });
  await connection.ready;
  calls.length = 0;
  return { connection, socket: Socket.current, publish, failed };
}
function broadcast(chains = true) {
  return {
    ...scope,
    type: 'broadcast',
    streamId: author,
    seq: '1',
    envelopeHash: encodeBinary(new Uint8Array(32)),
    envelope: 'AA',
    ...(chains ? { chains: [{ deviceId: author, chain: 'fixture' }] } : {}),
  };
}
it('verifies live author chains before envelope admission and Worker publication', async () => {
  const { connection, socket, publish, failed } = await open();
  try {
    socket.receive(broadcast());
    await connection.run(async () => {});
    expect(calls).toEqual(['chain', 'admit', 'fold']);
    expect(publish).toHaveBeenLastCalledWith({ source: 'updated', title: 'Title', ownData: false });
    expect(failed).not.toHaveBeenCalled();
    calls.length = 0;
    socket.receive(broadcast(false));
    await connection.run(async () => {});
    expect(calls).toEqual(['admit', 'fold']);
  } finally {
    connection.close();
  }
});
it('closes a forged live-chain broadcast before decoding or publishing its envelope', async () => {
  const { connection, socket, publish, failed } = await open(true);
  try {
    socket.receive(broadcast());
    await vi.waitFor(() => expect(failed).toHaveBeenCalledWith(new Error('Forged chain')));
    expect(calls).toEqual(['chain']);
    expect(publish).toHaveBeenCalledTimes(1);
    expect(socket.close).toHaveBeenCalledOnce();
  } finally {
    connection.close();
  }
});

it('waits for the exact records in the local writer projection before dependent reads', async () => {
  const { connection, socket, publish } = await open();
  const records = [
    { root: 'messages' as const, key: 'comment', value: { text: 'Hello', revision: 1 } },
  ];
  let completed = false;
  const pending = connection.waitForOwnRecords(records).then(() => {
    completed = true;
  });
  const own = (value: { text: string; revision: number }) => ({
    threads: {},
    intents: {},
    replies: {},
    messages: { comment: value },
  });
  try {
    projection.own = { foreign: own(records[0].value) };
    socket.receive(broadcast());
    await connection.run(async () => {});
    expect(completed).toBe(false);
    projection.own = { [author]: own({ text: 'Different', revision: 1 }) };
    socket.receive(broadcast());
    await connection.run(async () => {});
    expect(completed).toBe(false);
    // JSON object field order is cosmetic. Admission/fold and publication must precede resolution.
    projection.own = { [author]: own({ revision: 1, text: 'Hello' }) };
    socket.receive(broadcast());
    await pending;
    expect(publish).toHaveBeenLastCalledWith(expect.objectContaining({ own: projection.own }));
    await expect(connection.waitForOwnRecords(records)).resolves.toBeUndefined();
  } finally {
    connection.close();
  }
});

it('rejects an unobserved own publication on close without publishing it', async () => {
  const { connection, publish } = await open();
  const pending = connection.waitForOwnRecords([{ root: 'messages', key: 'missing', value: null }]);
  const rejected = expect(pending).rejects.toThrow('Disconnected');
  connection.close(new Error('Disconnected'));
  await rejected;
  expect(publish).toHaveBeenCalledTimes(1);
});

it('bounds own publication catchup and closes the unavailable connection', async () => {
  vi.useFakeTimers();
  const { connection, failed, socket, publish } = await open();
  const pending = connection.waitForOwnRecords([{ root: 'messages', key: 'missing', value: null }]);
  const rejected = expect(pending).rejects.toThrow('Own publication catchup timed out');
  await vi.advanceTimersByTimeAsync(10_000);
  await rejected;
  expect(connection.active).toBe(false);
  expect(failed).toHaveBeenCalledWith(new Error('Own publication catchup timed out'));
  expect(socket.close).toHaveBeenCalledOnce();
  expect(publish).toHaveBeenCalledTimes(1);
});
