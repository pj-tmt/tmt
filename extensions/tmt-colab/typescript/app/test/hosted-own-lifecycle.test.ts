import { afterEach, expect, it, vi } from 'vite-plus/test';
import type { SealedSyncPort } from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import type { Admission } from '../src/admission.js';
import type { Fold } from '../src/fold.js';
import { HostedPageReader } from '../src/hosted-reader.js';

afterEach(() => vi.unstubAllGlobals());
function fixture(author = true) {
  const calls: string[] = [],
    closeKeys = vi.fn(() => calls.push('keys-close'));
  const a = {
    epoch: '1',
    space: 'space',
    page: 'page',
    reader: {},
    registration: { deviceId: 'device' },
    closeKeys,
  } as unknown as Admission;
  const port = {
    context: { uid: 'uid', config: { projectId: 'demo-project' } },
    publish: vi.fn(),
    read: vi.fn(),
  } as unknown as SealedSyncPort;
  const fold = { close: vi.fn(() => calls.push('fold-close')) } as unknown as Fold;
  const reader = new HostedPageReader(
    port,
    a,
    { revision: '1', statement: new Uint8Array(32) },
    new Map(),
    fold,
    author ? () => new Uint8Array(32) : undefined,
  );
  return { reader, port, calls, closeKeys };
}
it('cancels and joins a writer waiting for its origin lock before releasing keys/port', async () => {
  let release!: () => void, aborted!: () => void;
  const held = new Promise<void>((r) => (release = r)),
    seen = new Promise<void>((r) => (aborted = r));
  vi.stubGlobal('navigator', {
    locks: {
      request: async (_key: string, options: { signal: AbortSignal }) => {
        await new Promise<void>((r) =>
          options.signal.addEventListener(
            'abort',
            () => {
              aborted();
              r();
            },
            { once: true },
          ),
        );
        await held;
        throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' };
      },
    },
  });
  const f = fixture(),
    writing = f.reader
      .submitOwn([], { signal: new AbortController().signal, deadlineAtMs: Date.now() + 30000 })
      .catch((e) => e);
  let joined = false;
  const closing = f.reader.close().then(() => {
    joined = true;
  });
  await seen;
  expect(joined).toBe(false);
  expect(f.closeKeys).not.toHaveBeenCalled();
  release();
  await closing;
  expect(await writing).toMatchObject({
    outcome: 'refused',
    confirmed: 0,
    failure: { state: 'cancelled' },
  });
  expect(f.calls).toEqual(['fold-close', 'keys-close']);
  expect(f.port.publish).not.toHaveBeenCalled();
  expect(f.port.read).not.toHaveBeenCalled();
});
it('has no publication fallback when fresh author admission was not supplied', async () => {
  const request = vi.fn();
  vi.stubGlobal('navigator', { locks: { request } });
  const f = fixture(false);
  await expect(
    f.reader.submitOwn([], {
      signal: new AbortController().signal,
      deadlineAtMs: Date.now() + 30000,
    }),
  ).rejects.toEqual({ code: 'REMOTE_STATE_UNAVAILABLE' });
  expect(request).not.toHaveBeenCalled();
  expect(f.port.publish).not.toHaveBeenCalled();
  await f.reader.close();
});
