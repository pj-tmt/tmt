import { afterEach, expect, it, vi } from 'vite-plus/test';
import { encodeBinary } from '@tmt/colab-client';
import type { Admission } from '../src/admission.js';
import { Connection } from '../src/connection.js';

const calls = vi.hoisted(() => [] as string[]);
vi.mock('../src/fold.js', () => ({
  Fold: class {
    async run({ updates }: { updates?: Uint8Array[] }) {
      calls.push('fold');
      return { source: updates?.length ? 'updated' : 'initial', title: 'Title' };
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
      return new Uint8Array([1]);
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
  constructor() {
    Socket.current = this;
  }
  receive(frame: Record<string, unknown>) {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
}
const page = '10000000-0000-4000-8000-000000000001';
const author = '30000000-0000-4000-8000-000000000001';
const scope = { version: 1, space: 'a'.repeat(32), page, epoch: '1' };
afterEach(() => vi.unstubAllGlobals());
async function open(rejectChain = false) {
  calls.length = 0;
  vi.stubGlobal('WebSocket', Socket);
  const admission = {
    ...scope,
    root: new Uint8Array(32),
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
