import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import { Admission } from '../src/admission.js';
import { Catchup } from '../src/catchup.js';
import { parseReaderFragment } from '../src/reader-link.js';
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
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));

const fields = {
  v: '1',
  space: v.space as string,
  page: v.page as string,
  link: v.linkId as string,
  rev: '4',
  st: c.encodeBinary(new Uint8Array(32).fill(9)),
  seed: c.encodeBinary(new Uint8Array(32).fill(7)),
};
const fragment = (overrides: Record<string, string | null> = {}, extra = '') =>
  '#' +
  Object.entries({ ...fields, ...overrides })
    .filter(([, value]) => value !== null)
    .map(([key, value]) => `${key}=${value}`)
    .join('&') +
  extra;

describe('reader link fragment', () => {
  it('parses the one grammar into canonical values', () => {
    const link = parseReaderFragment(fragment());
    expect(link).toMatchObject({ space: v.space, page: v.page, link: v.linkId, revision: '4' });
    expect(link.seed).toEqual(new Uint8Array(32).fill(7));
    expect(link.statement).toEqual(new Uint8Array(32).fill(9));
  });
  it('accepts the keys in any order', () => {
    const reordered =
      '#' +
      Object.entries(fields)
        .reverse()
        .map(([key, value]) => `${key}=${value}`)
        .join('&');
    expect(parseReaderFragment(reordered).revision).toBe('4');
  });
  const bad: [string, string][] = [
    ['empty', ''],
    ['no hash sign', fragment().slice(1)],
    ['other version', fragment({ v: '2' })],
    ['missing seed', fragment({ seed: null })],
    ['missing statement', fragment({ st: null })],
    ['duplicate key', fragment({}, '&rev=5')],
    ['unknown key', fragment({}, '&role=editor')],
    ['query-looking key', fragment({}, '&?x=1')],
    ['percent escape', fragment({ rev: '%34' })],
    ['space not canonical', fragment({ space: v.space.toUpperCase() })],
    ['page not a UUIDv4', fragment({ page: '10000000-0000-1000-8000-000000000001' })],
    ['link not a UUID', fragment({ link: 'nope' })],
    ['revision zero', fragment({ rev: '0' })],
    ['revision with sign', fragment({ rev: '+4' })],
    ['short seed', fragment({ seed: c.encodeBinary(new Uint8Array(31)) })],
    ['padded seed', fragment({ seed: fields.seed + '=' })],
    ['long statement', fragment({ st: c.encodeBinary(new Uint8Array(33)) })],
    ['oversized', fragment({}, '&' + 'x'.repeat(600))],
  ];
  it.each(bad)('rejects %s', (_name, hash) => {
    expect(() => parseReaderFragment(hash)).toThrow();
  });
});

describe('reader admission', () => {
  const root = hex(v.public);
  const membership = {
    revision: '1',
    statementHash: c.encodeBinary(hex(v.statementHash)),
    ownerKey: c.encodeBinary(root),
    statements: [c.encodeBinary(c.text(JSON.stringify(v.statement)))],
    more: false,
  };
  const catchup = JSON.stringify({
    version: 1,
    type: 'catchup',
    space: v.space,
    page: v.page,
    epoch: '1',
    membershipHead: membership,
    baseline: null,
    streams: [],
    more: true,
  });
  // A link device has a chain, but no member issuer statement to verify.
  const reader = () =>
    new Admission(
      v.space,
      v.page,
      '1',
      root,
      {
        deviceId: '30000000-0000-4000-8000-000000000001',
        chain: c.certificate.Chain.fromJson(c.text(JSON.stringify(v.chain))),
      } as Registration,
      { linkId: v.linkId, principal: '40000000-0000-4000-8000-000000000001' },
    );
  it('verifies the owner log but stores nothing in the owner app’s records', async () => {
    records.clear();
    const a = reader();
    await a.restore();
    await new Catchup(a, ['link', 'public']).admit(catchup);
    expect(a.head?.revision).toBe(1n);
    expect([...records.keys()]).toEqual([]);
  });
  it('refuses an owner key that does not derive the linked space', async () => {
    const wrong = new Admission(
      v.space,
      v.page,
      '1',
      hex(v.recipientSeed),
      { deviceId: v.device } as Registration,
      { linkId: v.linkId, principal: '40000000-0000-4000-8000-000000000001' },
    );
    await expect(wrong.restore()).rejects.toThrow();
  });
  it('opens only wraps addressed to its link, never a device wrap', async () => {
    const a = reader();
    await a.restore();
    await new Catchup(a, ['link', 'public']).admit(catchup);
    // The vector wrap is addressed to a device: a link reader must not take or open it.
    await expect(a.wraps([c.encodeBinary(c.text(JSON.stringify(v.wrap)))])).rejects.toThrow();
    expect(a.root).toBeNull();
  });
});
