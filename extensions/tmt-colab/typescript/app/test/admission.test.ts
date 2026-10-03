import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import { Admission, STATEMENT_ENVELOPE_BYTES } from '../src/admission.js';
import { Frames } from '../src/frames.js';
import { Catchup } from '../src/catchup.js';
import { Objects } from '../src/objects.js';
import type { Registration } from '../src/registration.js';

const records = new Map<string, unknown>();
let failWrite = false;
vi.mock('../src/storage.js', () => ({
  record: async (key: string, ...values: unknown[]) => {
    if (values.length) {
      if (failWrite) throw new Error('Fixture durable write failed');
      records.set(key, structuredClone(values[0]));
    } else return records.get(key);
  },
}));
vi.stubGlobal('navigator', { locks: { request: async (_key: string, fn: () => unknown) => fn() } });
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (v) => parseInt(v, 16));
const transport = (value: unknown) => c.encodeBinary(c.text(JSON.stringify(value)));
const raw = transport(v.statement);
const successor = c.statement.Envelope.fromJson(c.text(JSON.stringify(v.historyCases[0].envelope)));
async function secondHead() {
  return {
    ...head(),
    revision: '2',
    statementHash: c.encodeBinary(await successor.hash()),
    statements: [raw, c.encodeBinary(successor.toJson())],
  };
}
const root = hex(v.public);
function admission() {
  return new Admission(v.space, v.page, '1', root, {
    deviceId: v.page,
    chain: c.certificate.Chain.fromJson(c.text(JSON.stringify(v.chain))),
    issuer: c.statement.Envelope.fromJson(c.text(JSON.stringify(v.statement))),
  } as Registration);
}
function head() {
  return {
    revision: '1',
    statementHash: c.encodeBinary(hex(v.statementHash)),
    ownerKey: c.encodeBinary(root),
    statements: [raw],
    more: false,
  };
}
function first(membershipHead: unknown = head()) {
  return JSON.stringify({
    version: 1,
    type: 'catchup',
    space: v.space,
    page: v.page,
    epoch: '1',
    membershipHead,
    baseline: null,
    streams: [],
    more: true,
  });
}
it('persists a verified root/log before dependent state, and rejects lower heads/forks', async () => {
  records.clear();
  const a = admission();
  await new Catchup(a, 'private').admit(first(await secondHead()));
  expect(a.head?.revision).toBe(2n);
  expect(records.get(`log:${v.space}`)).toEqual((await secondHead()).statements);
  await expect(
    a.membership(
      {
        ...(await secondHead()),
        statements: [],
        statementHash: c.encodeBinary(new Uint8Array(32)),
      },
      true,
    ),
  ).rejects.toThrow();
  await expect(a.membership({ ...head(), statements: [] }, true)).rejects.toThrow();
  expect(a.head?.revision).toBe(2n);
});
it('rejects duplicate/unknown fields, wrong scope, incomplete logs and stream effects', async () => {
  records.clear();
  const bad = [
    first().replace('"version":1', '"version":1,"version":1'),
    first().replace('"baseline":null', '"baseline":null,"unexpected":true'),
    first().replace(`"page":"${v.page}"`, '"page":"00000000-0000-4000-8000-000000000099"'),
    first({ ...head(), statements: [] }),
    first().replace('"streams":[]', '"streams":[{}]'),
  ];
  for (const frame of bad)
    await expect(new Catchup(admission(), 'private').admit(frame)).rejects.toThrow();
});
it('accepts a valid owner author chain and rejects a forged chain with the same IDs', async () => {
  records.clear();
  vi.useFakeTimers();
  vi.setSystemTime(50);
  try {
    const a = admission();
    await new Catchup(a, 'private').admit(first());
    await a.chains([{ deviceId: v.page, chain: transport(v.chain) }]);
    expect(a.author(v.page, '1')).toEqual(root);
    expect(() => a.author(v.device, '1')).toThrow('Fresh membership');
    expect(() => a.author(v.page, '2')).toThrow('Fresh membership');
    const bad = { ...v.chain, issuerSignature: c.encodeBinary(new Uint8Array(64)) };
    await expect(a.chains([{ deviceId: v.page, chain: transport(bad) }])).rejects.toThrow();
  } finally {
    vi.useRealTimers();
  }
});

it('opens the independent device wrap only into a non-extractable HKDF handle', async () => {
  records.clear();
  const key = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b656e04220420'), hex(v.recipientSeed)),
    'X25519',
    false,
    ['deriveBits'],
  );
  const envelope = c.wrap.Envelope.fromJson(c.text(JSON.stringify(v.wrap)));
  const enc = await c.RecipientKey.fromHandle(key, envelope.header().recipientKey);
  const a = new Admission(v.space, v.page, '1', root, {
    deviceId: v.device,
    keys: { enc },
  } as Registration);
  await a.membership(await secondHead(), true);
  await a.wraps([transport(v.wrap)]);
  expect(a.root?.extractable).toBe(false);
  await expect(crypto.subtle.exportKey('raw', a.root!)).rejects.toThrow();
  const proof = new Uint8Array(
    await crypto.subtle.deriveBits(
      { name: 'HKDF', hash: 'SHA-256', salt: new Uint8Array(), info: c.text('proof') },
      a.root!,
      256,
    ),
  );
  expect(c.equal(proof, await c.hkdf(hex(v.epochKey), c.text('proof')))).toBe(true);
  await expect(
    a.wraps([transport({ ...v.wrap, signature: c.encodeBinary(new Uint8Array(64)) })]),
  ).rejects.toThrow();
});

it('reads only the exact revoked-device cut, not replacement checkpoints or offline tails', async () => {
  records.clear();
  vi.useFakeTimers();
  vi.setSystemTime(50);
  try {
    const a = admission();
    await new Catchup(a, 'private').admit(first(await secondHead()));
    await a.chains([{ deviceId: v.page, chain: transport(v.chain) }]);
    a.root = await crypto.subtle.importKey('raw', hex(v.epochKey), 'HKDF', false, ['deriveBits']);
    const signer = await crypto.subtle.importKey(
      'pkcs8',
      c.concat(hex('302e020100300506032b657004220420'), hex(v.seed)),
      'Ed25519',
      false,
      ['sign'],
    );
    const prefix = new Uint8Array(32).fill(4),
      ctx: c.Context = {
        space: v.space,
        page: v.page,
        epoch: '1',
        authorDevice: v.page,
        kind: 'checkpoint',
        namespace: 'content',
        membershipRevision: '1',
        streamSeq: '4',
        prevHash: prefix,
      };
    const seal = (context: c.Context) =>
      c.Envelope.seal(context, hex(v.epochKey), signer, new Uint8Array([255]));
    const cp = await seal(ctx),
      own = await seal({ ...ctx, namespace: 'own' });
    const five = await seal({ ...ctx, kind: 'update', streamSeq: '5' });
    const six = await seal({
      ...ctx,
      kind: 'update',
      namespace: 'own',
      streamSeq: '6',
      prevHash: await five.hash(),
    });
    const cuts = await Promise.all(
      [
        ['content', cp, five],
        ['own', own, six],
      ].map(async ([namespace, checkpoint, tail]) => {
        const cp = checkpoint as c.Envelope,
          t = tail as c.Envelope;
        return {
          pageId: v.page,
          epoch: '1',
          namespace,
          cut: c.encodeBinary(
            c.streamCut.input({
              streamId: v.page,
              namespace: namespace as string,
              checkpointHash: await cp.hash(),
              checkpointSeq: '4',
              tailHeadSeq: c.decodeHeader(t.header()).context.streamSeq,
              tailHeadHash: await t.hash(),
            }),
          ),
        };
      }),
    );
    const payload = c.text(JSON.stringify({ deviceId: v.page, cuts }));
    const input = c.statement.input({
      space: v.space,
      operation: 'device.revoke',
      revision: '3',
      previousHash: a.head!.hash,
      payloadDigest: await c.digest(payload),
    });
    const revoked = c.statement.Envelope.fromJson(
      c.text(
        JSON.stringify({
          statement: c.encodeBinary(input),
          payload: c.encodeBinary(payload),
          signature: c.encodeBinary(await c.sign(signer, input)),
        }),
      ),
    );
    await a.membership(
      {
        revision: '3',
        statementHash: c.encodeBinary(await revoked.hash()),
        ownerKey: c.encodeBinary(root),
        statements: [c.encodeBinary(revoked.toJson())],
        more: false,
      },
      true,
    );
    const entry = async (e: c.Envelope) => ({
      seq: c.decodeHeader(e.header()).context.streamSeq,
      envelopeHash: c.encodeBinary(await e.hash()),
      envelope: c.encodeBinary(e.toJson()),
    });
    const objects = new Objects(a);
    await objects.admit(v.page, await entry(cp), 'checkpoint', 'content');
    await objects.admit(v.page, await entry(own), 'checkpoint', 'own');
    expect(() => objects.finish()).toThrow();
    await objects.admit(v.page, await entry(five));
    await objects.admit(v.page, await entry(six));
    objects.finish();
    await expect(
      new Objects(a).admit(v.page, await entry(await seal(ctx)), 'checkpoint', 'content'),
    ).rejects.toThrow();
    await expect(
      objects.admit(
        v.page,
        await entry(
          await seal({ ...ctx, kind: 'update', streamSeq: '7', prevHash: await six.hash() }),
        ),
      ),
    ).rejects.toThrow();
    await expect(
      objects.admit(
        v.page,
        await entry(
          await seal({
            ...ctx,
            kind: 'update',
            streamSeq: '7',
            membershipRevision: '3',
            prevHash: await six.hash(),
          }),
        ),
      ),
    ).rejects.toThrow();
    expect(() => a.author(v.page, '1')).toThrow();
  } finally {
    vi.useRealTimers();
  }
});

// Independent root-signed fixture, using only the authority vector's public seed.
async function largeStatement(
  revision = '2',
  previousHash = hex(v.statementHash),
  titleSize = 250 * 1024,
) {
  const signer = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b657004220420'), hex(v.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
  const value = {
    pageId: v.page,
    epoch: '2',
    cuts: [],
    wraps: [],
    baseline: {
      pageId: v.page,
      epoch: '2',
      membershipRevision: revision,
      title: 'x'.repeat(titleSize),
      sourceDigest: c.encodeBinary(new Uint8Array(32)),
      baselineCommitment: c.encodeBinary(new Uint8Array(32)),
      objectEnvelopeHash: c.encodeBinary(new Uint8Array(32)),
    },
  };
  const payload = c.text(JSON.stringify(value)),
    input = c.statement.input({
      space: v.space,
      operation: 'epoch.advance',
      revision,
      previousHash,
      payloadDigest: await c.digest(payload),
    });
  const wire = {
    statement: c.encodeBinary(input),
    payload: c.encodeBinary(payload),
    signature: c.encodeBinary(await c.sign(signer, input)),
  };
  // Preserve transport spelling, including whitespace, rather than reserializing.
  const bytes = c.text('\n' + JSON.stringify(wire, null, 1) + '\n'),
    envelope = c.statement.Envelope.fromJson(bytes);
  return { bytes, hash: c.encodeBinary(await envelope.hash()), wire };
}
const scope = { version: 1, space: v.space, page: v.page, epoch: '1' };
function reference(hash: string, later = false, targetHash = hash) {
  const membership = { statements: [{ statementHash: hash }], more: false };
  return {
    ...scope,
    type: 'catchup',
    streams: [],
    more: true,
    ...(later
      ? { membership }
      : {
          baseline: null,
          membershipHead: {
            ...head(),
            ...membership,
            revision: '2',
            statementHash: targetHash,
          },
        }),
  };
}
function chunks(bytes: Uint8Array, hash: string) {
  const count = Math.ceil(bytes.length / 32768);
  return Array.from({ length: count }, (_, index) => ({
    ...scope,
    type: 'chunk',
    statementHash: hash,
    index,
    count,
    bytes: c.encodeBinary(bytes.slice(index * 32768, (index + 1) * 32768)),
  }));
}
it('admits fresh/later and resumed/first statement references only after exact verified reassembly', async () => {
  const f = await largeStatement();
  expect(f.bytes.length).toBeGreaterThan(64 * 1024);
  expect(chunks(f.bytes, f.hash).length).toBeGreaterThan(8);
  expect(f.bytes).not.toEqual(c.statement.Envelope.fromJson(f.bytes).toJson());
  for (const later of [false, true]) {
    records.clear();
    const a = admission(),
      catchup = new Catchup(a, 'private'),
      fail = vi.fn(),
      frames = new Frames(a, fail);
    try {
      if (later)
        await catchup.admit(first({ ...head(), revision: '2', statementHash: f.hash, more: true }));
      else await a.membership(head(), true);
      const before = records.get(`log:${v.space}`);
      expect(frames.receive(JSON.stringify(reference(f.hash, later)))).toBeNull();
      for (const [index, chunk] of chunks(f.bytes, f.hash).entries()) {
        const complete = frames.receive(JSON.stringify(chunk));
        if (index + 1 < chunk.count) {
          expect(complete).toBeNull();
          expect(a.head?.revision).toBe(1n);
          expect(records.get(`log:${v.space}`)).toEqual(before);
        } else {
          expect(complete).not.toBeNull();
          await catchup.admitValue(complete);
        }
      }
      expect(a.head?.revision).toBe(2n);
      expect(c.encodeBinary(a.head!.hash)).toBe(f.hash);
      expect(records.get(`log:${v.space}`)).toEqual([raw, c.encodeBinary(f.bytes)]);
      expect(fail).not.toHaveBeenCalled();
      vi.useFakeTimers();
      vi.setSystemTime(50);
      const restored = admission();
      await restored.restore();
      expect(restored.head).toEqual(a.head);
    } finally {
      frames.close();
      vi.useRealTimers();
    }
  }
});
it('rejects signed-chain, payload, signature, reference and target substitutions without publishing or persisting', async () => {
  const valid = await largeStatement(),
    wrongPrevious = await largeStatement('2', new Uint8Array(32)),
    wrongRevision = await largeStatement('3');
  const payload = JSON.parse(c.decodeText(c.binary(valid.wire.payload, c.payload.MAX_BYTES)));
  payload.baseline.title = 'y' + payload.baseline.title.slice(1);
  const changedPayload = c.text(JSON.stringify({ ...valid.wire, payload: transport(payload) }));
  const forged = c.text(
    JSON.stringify({ ...valid.wire, signature: c.encodeBinary(new Uint8Array(64)) }),
  );
  const forgedHash = c.encodeBinary(await c.statement.Envelope.fromJson(forged).hash());
  for (const [bytes, hash, target] of [
    [valid.bytes, c.encodeBinary(new Uint8Array(32)), valid.hash],
    [changedPayload, valid.hash, valid.hash],
    [forged, forgedHash, forgedHash],
    [wrongPrevious.bytes, wrongPrevious.hash, wrongPrevious.hash],
    [wrongRevision.bytes, wrongRevision.hash, wrongRevision.hash],
    [valid.bytes, valid.hash, c.encodeBinary(new Uint8Array(32))],
  ] as [Uint8Array, string, string][]) {
    records.clear();
    const a = admission();
    await a.membership(head(), true);
    const before = a.head,
      durable = records.get(`log:${v.space}`),
      frames = new Frames(a, vi.fn());
    try {
      frames.receive(JSON.stringify(reference(hash, false, target)));
      let complete = null;
      for (const chunk of chunks(bytes, hash)) complete = frames.receive(JSON.stringify(chunk));
      await expect(new Catchup(a, 'private').admitValue(complete)).rejects.toThrow();
      expect(a.head).toEqual(before);
      expect(records.get(`log:${v.space}`)).toEqual(durable);
      // Rejection did not corrupt the candidate target or previous head.
      await a.membership({ ...head(), statements: [] }, true);
    } finally {
      frames.close();
    }
  }
});
it('retains the previous head when a durable prefix conflicts or persistence fails', async () => {
  const f = await largeStatement();
  for (const failure of ['prefix', 'write']) {
    records.clear();
    const a = admission();
    await a.membership(head(), true);
    if (failure === 'prefix') records.set(`log:${v.space}`, ['fork']);
    else failWrite = true;
    const before = records.get(`log:${v.space}`),
      frames = new Frames(a, vi.fn());
    try {
      frames.receive(JSON.stringify(reference(f.hash)));
      let complete = null;
      for (const chunk of chunks(f.bytes, f.hash)) complete = frames.receive(JSON.stringify(chunk));
      await expect(new Catchup(a, 'private').admitValue(complete)).rejects.toThrow();
      expect(a.head?.revision).toBe(1n);
      expect(records.get(`log:${v.space}`)).toEqual(before);
    } finally {
      frames.close();
      failWrite = false;
    }
  }
});
it('stages an entire inline page and refuses unassembled references', async () => {
  records.clear();
  const a = admission(),
    second = await secondHead(),
    forged = JSON.parse(c.decodeText(successor.toJson()));
  forged.signature = c.encodeBinary(new Uint8Array(64));
  await expect(
    a.membership({ ...second, statements: [raw, transport(forged)] }, true),
  ).rejects.toThrow();
  expect(a.head).toBeNull();
  expect(records.get(`log:${v.space}`)).toBeUndefined();
  await expect(
    a.membership({ ...head(), statements: [{ statementHash: head().statementHash }] }, true),
  ).rejects.toThrow();
  expect(a.head).toBeNull();
});
it('rejects statement identity, order, bounds and interleaving, discarding partial assemblies', async () => {
  const f = await largeStatement(),
    part = chunks(f.bytes, f.hash)[0];
  expect(STATEMENT_ENVELOPE_BYTES).toBe(1_051_989);
  for (const change of [
    { epoch: '2' },
    { space: 'wrong' },
    { page: v.device },
    { statementHash: c.encodeBinary(new Uint8Array(32)) },
    { objectId: 'a'.repeat(64) },
    { envelopeHash: f.hash },
    { extra: true },
    { index: 1 },
    { index: -1 },
    { index: 0.5 },
    { count: 0 },
    { count: 34 },
    { count: 1.5 },
    { bytes: '' },
    { bytes: c.encodeBinary(new Uint8Array(1)) },
  ]) {
    const frames = new Frames(admission(), vi.fn());
    frames.receive(JSON.stringify(reference(f.hash)));
    expect(() => frames.receive(JSON.stringify({ ...part, ...change }))).toThrow();
    expect(() => frames.receive(JSON.stringify(part))).toThrow();
    frames.close();
  }
  for (const malformed of [
    {
      ...reference(f.hash),
      membershipHead: { ...head(), statements: [{ statementHash: f.hash }, raw] },
    },
    {
      ...reference(f.hash),
      baseline: raw,
      baselineObject: { envelopeHash: f.hash, envelope: { objectId: 'a'.repeat(64) } },
    },
    {
      ...reference(f.hash),
      membershipHead: { ...head(), statements: [{ statementHash: f.hash, extra: true }] },
    },
    { ...reference(f.hash), wraps: [] },
  ]) {
    const frames = new Frames(admission(), vi.fn());
    expect(() => frames.receive(JSON.stringify(malformed))).toThrow();
    frames.close();
  }
  const frames = new Frames(admission(), vi.fn());
  frames.receive(JSON.stringify(reference(f.hash)));
  frames.receive(JSON.stringify(part));
  expect(() =>
    frames.receive(JSON.stringify({ ...part, index: 1, count: part.count + 1 })),
  ).toThrow();
  frames.receive(JSON.stringify(reference(f.hash)));
  expect(() => frames.receive(first())).toThrow();
  frames.close();
});
it('enforces the aggregate statement cap, absolute deadline and explicit close cleanup', async () => {
  const f = await largeStatement(),
    fail = vi.fn(),
    frames = new Frames(admission(), fail);
  try {
    frames.receive(JSON.stringify(reference(f.hash)));
    const oversized = chunks(new Uint8Array(STATEMENT_ENVELOPE_BYTES + 1), f.hash);
    for (const part of oversized.slice(0, -1)) frames.receive(JSON.stringify(part));
    expect(() => frames.receive(JSON.stringify(oversized.at(-1)))).toThrow();
    vi.useFakeTimers();
    frames.receive(JSON.stringify(reference(f.hash)));
    await vi.advanceTimersByTimeAsync(1500);
    frames.receive(JSON.stringify(chunks(f.bytes, f.hash)[0]));
    await vi.advanceTimersByTimeAsync(500);
    expect(fail).toHaveBeenCalledOnce();
    expect(() => frames.receive(JSON.stringify(chunks(f.bytes, f.hash)[1]))).toThrow();
    frames.receive(JSON.stringify(reference(f.hash)));
    frames.close();
    await vi.advanceTimersByTimeAsync(2000);
    expect(fail).toHaveBeenCalledOnce();
    expect(() => frames.receive(JSON.stringify(chunks(f.bytes, f.hash)[0]))).toThrow();
  } finally {
    frames.close();
    vi.useRealTimers();
  }
});
