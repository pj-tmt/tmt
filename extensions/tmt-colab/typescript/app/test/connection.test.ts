import { afterEach, expect, it, vi } from 'vite-plus/test';
import { binary, concat, digest, encodeBinary, text } from '@tmt/colab-client';
import type { Admission } from '../src/admission.js';
import { Connection } from '../src/connection.js';
import type { OwnState } from '../src/fold-protocol.js';
import { SAVE_SOURCE_BYTES, SaveTooLarge } from '../src/save.js';

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
    closeKeys() {},
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

const operation = '40000000-0000-4000-8000-000000000001';
/** Frames the page asked for; acknowledgements of what it received are not requests. */
const sent = (socket: Socket) =>
  socket.send.mock.calls
    .map(([raw]) => JSON.parse(raw as string) as Record<string, unknown>)
    .filter((frame) => frame.type !== 'ack');
const savedFrame = (state: Record<string, unknown>) => ({
  ...scope,
  type: 'saveresult',
  operationId: operation,
  ...state,
});
const awaitSent = (socket: Socket, count: number) =>
  vi.waitFor(() => expect(sent(socket).length).toBeGreaterThanOrEqual(count));

it('saves a small source inline, bound to the base and source digests, and resolves on the reply', async () => {
  const { connection, socket, failed } = await open();
  try {
    const pending = connection.save(operation, '<p>old</p>', '<p>new</p>');
    await awaitSent(socket, 1);
    expect(sent(socket)).toEqual([
      {
        ...scope,
        type: 'save',
        operationId: operation,
        baseSha256: encodeBinary(await digest(text('<p>old</p>'))),
        sourceSha256: encodeBinary(await digest(text('<p>new</p>'))),
        source: encodeBinary(text('<p>new</p>')),
      },
    ]);
    socket.receive(savedFrame({ state: 'committed', revision: 'v1:00ff' }));
    await expect(pending).resolves.toEqual({
      operationId: operation,
      state: 'committed',
      revision: 'v1:00ff',
    });
    expect(failed).not.toHaveBeenCalled();
  } finally {
    connection.close();
  }
});

it('uploads a large source as a reference and ordered 32 KiB chunks that rebuild it exactly', async () => {
  const { connection, socket } = await open();
  try {
    const source = 'é'.repeat(100_000) + '<p>end</p>';
    const bytes = text(source);
    const pending = connection.save(operation, 'old', source);
    const count = Math.ceil(bytes.length / (32 * 1024));
    await awaitSent(socket, 1 + count);
    const [save, ...chunks] = sent(socket);
    const hash = encodeBinary(await digest(bytes));
    expect(save).toMatchObject({ type: 'save', sourceSha256: hash });
    const objectId = (save.source as { objectId: string }).objectId;
    expect(objectId).toMatch(/^[0-9a-f]{64}$/);
    expect(chunks).toHaveLength(count);
    chunks.forEach((chunk, index) =>
      expect(chunk).toMatchObject({ type: 'chunk', objectId, envelopeHash: hash, index, count }),
    );
    expect(concat(...chunks.map((chunk) => binary(chunk.bytes as string, 32 * 1024)))).toEqual(
      bytes,
    );
    socket.receive(savedFrame({ state: 'unchanged', revision: 'v1:3a3a' }));
    await expect(pending).resolves.toMatchObject({ state: 'unchanged', revision: 'v1:3a3a' });
  } finally {
    connection.close();
  }
});

it('refuses a source over the page limit before sending anything, naming both numbers', async () => {
  const { connection, socket } = await open();
  try {
    const rejected = await connection
      .save(operation, 'old', 'x'.repeat(SAVE_SOURCE_BYTES + 1))
      .catch((error) => error);
    expect(rejected).toBeInstanceOf(SaveTooLarge);
    expect(rejected).toMatchObject({ size: SAVE_SOURCE_BYTES + 1, limit: SAVE_SOURCE_BYTES });
    expect(sent(socket)).toEqual([]);
  } finally {
    connection.close();
  }
});

it('carries every refusal reply with its stable code and message', async () => {
  const { connection, socket } = await open();
  try {
    const pending = connection.save(operation, 'old', 'new');
    await awaitSent(socket, 1);
    socket.receive(savedFrame({ state: 'rejected', code: 'COLAB_STALE_BASE', message: 'Moved.' }));
    await expect(pending).resolves.toEqual({
      operationId: operation,
      state: 'rejected',
      code: 'COLAB_STALE_BASE',
      message: 'Moved.',
    });
  } finally {
    connection.close();
  }
});

it('allows one save or status request at a time per connection', async () => {
  const { connection, socket } = await open();
  try {
    const first = connection.save(operation, 'old', 'new');
    await awaitSent(socket, 1);
    await expect(connection.saveStatus(operation)).rejects.toThrow();
    socket.receive(savedFrame({ state: 'committed', revision: '1' }));
    await first;
    const status = connection.saveStatus(operation);
    await awaitSent(socket, 2);
    expect(sent(socket).at(-1)).toEqual({ ...scope, type: 'savestatus', operationId: operation });
    socket.receive(savedFrame({ state: 'absent' }));
    await expect(status).resolves.toEqual({ operationId: operation, state: 'absent' });
  } finally {
    connection.close();
  }
});

it('closes on a reply for another operation or one nobody asked for', async () => {
  for (const reply of [
    savedFrame({
      state: 'committed',
      revision: '1',
      operationId: '40000000-0000-4000-8000-000000000002',
    }),
    savedFrame({ state: 'committed' }),
    savedFrame({ state: 'rejected', code: 'lowercase' }),
  ]) {
    const { connection, socket, failed } = await open();
    const pending = connection.save(operation, 'old', 'new');
    const rejected = expect(pending).rejects.toThrow();
    await awaitSent(socket, 1);
    socket.receive(reply);
    await rejected;
    expect(failed).toHaveBeenCalledOnce();
    expect(connection.active).toBe(false);
  }
  const { connection, socket, failed } = await open();
  socket.receive(savedFrame({ state: 'committed', revision: '1' }));
  await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
  connection.close();
});

it('rejects an unanswered save when the connection closes and when the reply budget runs out', async () => {
  const lost = await open();
  const pending = lost.connection.save(operation, 'old', 'new');
  const rejected = expect(pending).rejects.toThrow('Sync disconnected');
  await awaitSent(lost.socket, 1);
  lost.connection.close(new Error('Sync disconnected'));
  await rejected;

  vi.useFakeTimers();
  const slow = await open();
  const waiting = slow.connection.save(operation, 'old', 'new');
  const timedOut = expect(waiting).rejects.toThrow('Save reply timed out');
  await awaitSent(slow.socket, 1);
  await vi.advanceTimersByTimeAsync(30_000);
  await timedOut;
  expect(slow.connection.active).toBe(false);
});
