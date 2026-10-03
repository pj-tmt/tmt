import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import { Admission } from '../src/admission.js';
import { Catchup } from '../src/catchup.js';
import type { Registration } from '../src/registration.js';

const records = new Map<string, unknown>();
vi.mock('../src/storage.js', () => ({
  record: async (key: string, ...values: unknown[]) => {
    if (values.length) records.set(key, structuredClone(values[0]));
    else return records.get(key);
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
  return new Admission(v.space, v.page, '1', root, { deviceId: v.device } as Registration);
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
