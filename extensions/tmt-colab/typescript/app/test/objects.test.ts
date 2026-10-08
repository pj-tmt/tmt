import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import { Objects, type ObjectEntry } from '../src/objects.js';
import { Frames } from '../src/frames.js';
import type { Admission } from '../src/admission.js';
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/model-v1.json', import.meta.url), 'utf8'),
);
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));
async function fixture() {
  const context = c.decodeHeader(hex(v.header)).context;
  context.streamSeq = '1';
  context.prevHash = new Uint8Array(32);
  const root = await crypto.subtle.importKey('raw', hex(v.master), 'HKDF', false, ['deriveBits']);
  const signer = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b657004220420'), hex(v.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
  const admission = {
    ...context,
    root,
    readRoot: (epoch: string) => {
      c.requireValue(epoch === context.epoch && admission.root !== null);
      return admission.root!;
    },
    readAuthor: () => ({ key: hex(v.public), ownerDevice: true }),
    cuts: () => [],
  } as unknown as Admission;
  const seal = (ctx = context, bytes = new Uint8Array([1, 2])) =>
    c.Envelope.seal(ctx, root, signer, bytes);
  const entry = async (envelope: c.Envelope): Promise<ObjectEntry> => ({
    seq: c.decodeHeader(envelope.header()).context.streamSeq,
    envelopeHash: c.encodeBinary(await envelope.hash()),
    envelope: c.encodeBinary(envelope.toJson()),
  });
  return { context, admission, seal, entry };
}
it('verifies scope/signature/hash and chain continuity; exact replay applies nothing', async () => {
  const f = await fixture(),
    objects = new Objects(f.admission),
    one = await f.seal(),
    row = await f.entry(one);
  expect(await objects.admit(f.context.authorDevice, row)).toEqual({
    namespace: 'content',
    writer: f.context.authorDevice,
    update: new Uint8Array([1, 2]),
  });
  expect(await objects.admit(f.context.authorDevice, row)).toBeNull();
  await expect(
    objects.admit(f.context.authorDevice, {
      ...row,
      envelopeHash: c.encodeBinary(new Uint8Array(32)),
    }),
  ).rejects.toThrow();
  const wrong = await f.seal({ ...f.context, streamSeq: '2', prevHash: new Uint8Array(32) });
  await expect(objects.admit(f.context.authorDevice, await f.entry(wrong))).rejects.toThrow();
  const two = await f.seal({ ...f.context, streamSeq: '2', prevHash: await one.hash() });
  expect(await objects.admit(f.context.authorDevice, await f.entry(two))).toEqual({
    namespace: 'content',
    writer: f.context.authorDevice,
    update: new Uint8Array([1, 2]),
  });
  const conflict = await f.seal(
    { ...f.context, streamSeq: '2', prevHash: await one.hash() },
    new Uint8Array([3]),
  );
  await expect(objects.admit(f.context.authorDevice, await f.entry(conflict))).rejects.toThrow();
  const forged = JSON.parse(c.decodeText(one.toJson()));
  forged.signature = c.encodeBinary(new Uint8Array(64));
  const envelope = c.Envelope.fromJson(c.text(JSON.stringify(forged)));
  await expect(
    new Objects(f.admission).admit(f.context.authorDevice, await f.entry(envelope)),
  ).rejects.toThrow();
});
it('opens own ciphertext only after admission, and rejects author substitution and gaps', async () => {
  const f = await fixture(),
    objects = new Objects(f.admission);
  const gap = await f.seal({ ...f.context, streamSeq: '3' });
  await expect(objects.admit(f.context.authorDevice, await f.entry(gap))).rejects.toThrow();
  const own = await f.seal({ ...f.context, namespace: 'own' });
  const root = f.admission.root;
  f.admission.root = null;
  await expect(objects.admit(f.context.authorDevice, await f.entry(own))).rejects.toThrow();
  expect(objects.head(f.context.authorDevice).seq).toBe(0n);
  f.admission.root = root;
  expect(await objects.admit(f.context.authorDevice, await f.entry(own))).toEqual({
    namespace: 'own',
    writer: f.context.authorDevice,
    update: new Uint8Array([1, 2]),
  });
  expect(objects.ownData).toBe(true);
  expect(objects.head(f.context.authorDevice).seq).toBe(1n);
  await expect(
    objects.streams([
      { streamId: f.context.authorDevice, namespace: 'content', checkpoint: {}, tail: [] },
    ]),
  ).rejects.toThrow();
  await expect(
    objects.admit('00000000-0000-4000-8000-000000000123', await f.entry(await f.seal())),
  ).rejects.toThrow();
});
it('reassembles exact bounded chunks, binds referenced ID and enforces an absolute deadline', async () => {
  const f = await fixture(),
    fail = vi.fn(),
    frames = new Frames(f.admission, fail),
    env = await f.seal(f.context, new Uint8Array(50_000)),
    row = await f.entry(env);
  const scope = {
      version: 1,
      space: f.context.space,
      page: f.context.page,
      epoch: f.context.epoch,
    },
    id = c.decodeHeader(env.header()).objectId;
  const reference = {
    ...scope,
    type: 'broadcast',
    streamId: f.context.authorDevice,
    ...row,
    envelope: { objectId: id },
  };
  expect(frames.receive(JSON.stringify(reference))).toBeNull();
  const bytes = env.toJson(),
    count = Math.ceil(bytes.length / 32768);
  let complete: Record<string, unknown> | null = null;
  for (let index = 0; index < count; index++)
    complete = frames.receive(
      JSON.stringify({
        ...scope,
        type: 'chunk',
        objectId: id,
        envelopeHash: row.envelopeHash,
        index,
        count,
        bytes: c.encodeBinary(bytes.slice(index * 32768, (index + 1) * 32768)),
      }),
    );
  expect(complete?.envelope).toBe(row.envelope);
  expect(fail).not.toHaveBeenCalled();
  vi.useFakeTimers();
  try {
    frames.receive(JSON.stringify(reference));
    await vi.advanceTimersByTimeAsync(2000);
    expect(fail).toHaveBeenCalledOnce();
    expect(() =>
      frames.receive(
        JSON.stringify({
          ...scope,
          type: 'chunk',
          objectId: id,
          envelopeHash: row.envelopeHash,
          index: 0,
          count,
          bytes: c.encodeBinary(bytes.slice(0, 32768)),
        }),
      ),
    ).toThrow();
  } finally {
    frames.close();
    vi.useRealTimers();
  }
});

it('paired checkpoints bind one prefix and preserve the cross-namespace tail chain', async () => {
  const f = await fixture(),
    objects = new Objects(f.admission),
    prefix = new Uint8Array(32).fill(4);
  const cp = await f.seal({ ...f.context, kind: 'checkpoint', streamSeq: '4', prevHash: prefix });
  const own = await f.seal({
    ...f.context,
    kind: 'checkpoint',
    namespace: 'own',
    streamSeq: '4',
    prevHash: prefix,
  });
  expect(
    await objects.admit(f.context.authorDevice, await f.entry(cp), 'checkpoint', 'content'),
  ).toEqual({
    namespace: 'content',
    writer: f.context.authorDevice,
    update: new Uint8Array([1, 2]),
  });
  expect(
    await objects.admit(f.context.authorDevice, await f.entry(own), 'checkpoint', 'own'),
  ).toEqual({ namespace: 'own', writer: f.context.authorDevice, update: new Uint8Array([1, 2]) });
  expect(objects.head(f.context.authorDevice).hash).toEqual(prefix);
  const tailOwn = await f.seal({
    ...f.context,
    namespace: 'own',
    streamSeq: '5',
    prevHash: prefix,
  });
  const tailContent = await f.seal({
    ...f.context,
    streamSeq: '6',
    prevHash: await tailOwn.hash(),
  });
  expect(await objects.admit(f.context.authorDevice, await f.entry(tailOwn))).toEqual({
    namespace: 'own',
    writer: f.context.authorDevice,
    update: new Uint8Array([1, 2]),
  });
  expect(await objects.admit(f.context.authorDevice, await f.entry(tailContent))).toEqual({
    namespace: 'content',
    writer: f.context.authorDevice,
    update: new Uint8Array([1, 2]),
  });
  expect(objects.cursors().map((x) => [x.namespace, x.seq])).toEqual([
    ['content', '6'],
    ['own', '5'],
  ]);
  objects.finish();
});
it('rejects checkpoint prefix/namespace substitutions and replacement envelopes', async () => {
  const f = await fixture();
  for (const change of [{ streamSeq: '3' }, { prevHash: new Uint8Array(32).fill(9) }]) {
    const objects = new Objects(f.admission),
      cp = await f.seal({
        ...f.context,
        kind: 'checkpoint',
        streamSeq: '4',
        prevHash: new Uint8Array(32).fill(4),
      });
    await objects.admit(f.context.authorDevice, await f.entry(cp), 'checkpoint', 'content');
    const wrong = await f.seal({
      ...f.context,
      kind: 'checkpoint',
      namespace: 'own',
      streamSeq: '4',
      prevHash: new Uint8Array(32).fill(4),
      ...change,
    });
    await expect(
      objects.admit(f.context.authorDevice, await f.entry(wrong), 'checkpoint', 'own'),
    ).rejects.toThrow();
  }
  const objects = new Objects(f.admission),
    cp = await f.seal({ ...f.context, kind: 'checkpoint' });
  await expect(
    objects.admit(f.context.authorDevice, await f.entry(cp), 'checkpoint', 'own'),
  ).rejects.toThrow();
  await objects.admit(f.context.authorDevice, await f.entry(cp), 'checkpoint', 'content');
  const replacement = await f.seal({ ...f.context, kind: 'checkpoint' });
  await expect(
    objects.admit(f.context.authorDevice, await f.entry(replacement), 'checkpoint', 'content'),
  ).rejects.toThrow();
});
