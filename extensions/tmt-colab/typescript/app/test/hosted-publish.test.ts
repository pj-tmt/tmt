import { expect, it, vi } from 'vite-plus/test';
import type {
  SealedSyncPort,
  SyncCall,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import { publishHostedOwn } from '../src/hosted-publish.js';

const call = (): SyncCall => ({
  signal: new AbortController().signal,
  deadlineAtMs: Date.now() + 30_000,
});
const document = (id = 'original-1') => ({
  target: { resource: 'log', id },
  bytes: new Uint8Array([1, 2, 3]),
  digest: 'a'.repeat(64),
});
function fixture() {
  const publish = vi.fn<SealedSyncPort['publish']>(async () => ({ kind: 'confirmed' }));
  const read = vi.fn<SealedSyncPort['read']>(async (target) => document(target.id));
  const port = { publish, read } as unknown as SealedSyncPort;
  return { port, publish, read };
}
it('publishes a frozen original once and confirms with exactly one same-ID readback', async () => {
  const f = fixture(),
    input = document();
  f.publish.mockImplementation(async () => {
    input.bytes.fill(9);
    input.target.id = 'changed-by-caller';
    return { kind: 'confirmed' };
  });
  expect(await publishHostedOwn(f.port, [input], call())).toEqual({
    outcome: 'sent',
    confirmed: 1,
  });
  expect(f.publish).toHaveBeenCalledOnce();
  expect(f.publish.mock.calls[0][0]).toMatchObject({ mode: 'create', document: document() });
  expect(f.read).toHaveBeenCalledExactlyOnceWith(
    { resource: 'log', id: 'original-1' },
    expect.anything(),
  );
});
it.each(['REMOTE_SCOPE_DENIED', 'REMOTE_BUDGET_EXHAUSTED', 'REMOTE_RATE_LIMITED'])(
  'does not read, resend or refund a proved pre-effect %s refusal',
  async (code) => {
    const f = fixture();
    f.publish.mockRejectedValue({ code });
    expect(await publishHostedOwn(f.port, [document()], call())).toMatchObject({
      outcome: 'refused',
      confirmed: 0,
      failure: { code },
    });
    expect(f.publish).toHaveBeenCalledOnce();
    expect(f.read).not.toHaveBeenCalled();
  },
);
it.each(['missing', 'unreadable', 'different-bytes', 'different-digest', 'wrong-target'])(
  'settles the %s exact-ID observation without resealing or resending',
  async (reason) => {
    const f = fixture();
    f.publish.mockResolvedValue({ kind: 'unknown' });
    f.read.mockImplementation(async () => {
      if (reason === 'unreadable') throw new Error('PRIVATE_PROVIDER_DETAIL');
      if (reason === 'missing') return undefined;
      const d = document();
      if (reason === 'different-bytes') d.bytes = new Uint8Array([1, 2, 4]);
      if (reason === 'different-digest') d.digest = 'b'.repeat(64);
      if (reason === 'wrong-target') d.target.id = 'other';
      return d;
    });
    expect(await publishHostedOwn(f.port, [document()], call())).toMatchObject({
      outcome:
        reason === 'different-bytes' || reason === 'different-digest'
          ? 'writer-in-use'
          : 'unconfirmed',
      confirmed: 0,
    });
    expect(f.publish).toHaveBeenCalledOnce();
    expect(f.read).toHaveBeenCalledOnce();
  },
);
it('settles a lost reply only from the exact original and never reissues the create', async () => {
  const f = fixture();
  f.publish.mockResolvedValue({ kind: 'unknown' });
  expect(await publishHostedOwn(f.port, [document()], call())).toEqual({
    outcome: 'sent',
    confirmed: 1,
  });
  expect(f.publish).toHaveBeenCalledOnce();
  expect(f.read).toHaveBeenCalledOnce();
});
it('does not add another read after the SDK already proved identical existing bytes', async () => {
  const f = fixture();
  f.publish.mockResolvedValue({ kind: 'already-present' });
  expect(await publishHostedOwn(f.port, [document()], call())).toEqual({
    outcome: 'sent',
    confirmed: 1,
  });
  expect(f.read).not.toHaveBeenCalled();
});
it('never labels a partially confirmed multi-document request fully sent or wholly refused', async () => {
  const f = fixture();
  // First document confirmed, second refused; the third is never attempted.
  f.publish
    .mockResolvedValueOnce({ kind: 'confirmed' })
    .mockRejectedValueOnce({ code: 'REMOTE_SCOPE_DENIED' });
  expect(
    await publishHostedOwn(f.port, [document('one'), document('two'), document('three')], call()),
  ).toMatchObject({ outcome: 'unconfirmed', confirmed: 1 });
  expect(f.publish).toHaveBeenCalledTimes(2);
  expect(f.read).toHaveBeenCalledOnce();
});
it.each([0, 5])('refuses %i documents before the first effect', async (count) => {
  const f = fixture();
  await expect(
    publishHostedOwn(
      f.port,
      Array.from({ length: count }, (_, i) => document(String(i))),
      call(),
    ),
  ).rejects.toMatchObject({ code: 'REMOTE_INPUT_INVALID' });
  expect(f.publish).not.toHaveBeenCalled();
  expect(f.read).not.toHaveBeenCalled();
});
it.each(['checkpoints', 'authority', 'members'])(
  'the own publisher never accepts %s documents',
  async (resource) => {
    const f = fixture(),
      d = document();
    d.target.resource = resource;
    await expect(publishHostedOwn(f.port, [d], call())).rejects.toMatchObject({
      code: 'REMOTE_SCOPE_DENIED',
    });
    expect(f.publish).not.toHaveBeenCalled();
  },
);

it('preserves an already-existing different original as definitive conflict with no second read or reseal', async () => {
  const f = fixture();
  f.publish.mockRejectedValue({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'conflict' });
  expect(await publishHostedOwn(f.port, [document()], call())).toEqual({
    outcome: 'writer-in-use',
    confirmed: 0,
    failure: { code: 'REMOTE_STATE_UNAVAILABLE', state: 'conflict' },
  });
  expect(f.publish).toHaveBeenCalledOnce();
  expect(f.read).not.toHaveBeenCalled();
});

it('never reconciles a declared unsupported create into a success', async () => {
  const f = fixture();
  f.publish.mockRejectedValue({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'unsupported' });
  expect(await publishHostedOwn(f.port, [document()], call())).toMatchObject({
    outcome: 'refused',
    confirmed: 0,
    failure: { state: 'unsupported' },
  });
  expect(f.read).not.toHaveBeenCalled();
  expect(f.publish).toHaveBeenCalledOnce();
});
