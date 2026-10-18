import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import { Admission } from '../src/admission.js';
import { admitHostedMembership, HostedReshareRequired } from '../src/hosted-authority.js';
import type { Registration } from '../src/registration.js';

vi.mock('../src/storage.js', () => ({
  record: () => {
    throw new Error('A link reader must never persist membership or keys');
  },
}));
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (value: string) =>
  Uint8Array.from(value.match(/../g) ?? [], (byte) => parseInt(byte, 16));
const raw = c.text(' \n' + JSON.stringify(v.statement) + '\n');
const successor = c.text(JSON.stringify(v.historyCases[0].envelope));
const pin = () => ({ revision: '1', statement: hex(v.statementHash) });
function admission(root = hex(v.public)) {
  return new Admission(v.space, v.page, '1', root, {} as Registration, {
    linkId: v.linkId,
    principal: 'holder-uid',
    expiresAt: Date.parse('2027-01-01T00:00:00Z'),
  });
}

it('admits the independent root-signed chain in revision order through existing Admission', async () => {
  const a = admission();
  await a.restore();
  await admitHostedMembership(a, [successor, raw], pin());
  expect(a.head?.revision).toBe(2n);
  expect(a.statements().map((statement) => statement.header.revision)).toEqual(['1', '2']);
});

it('accepts an unchanged verified prefix and appends only its new signed successor', async () => {
  const a = admission();
  await admitHostedMembership(a, [raw], pin());
  await admitHostedMembership(a, [raw], pin());
  expect(a.statements()).toHaveLength(1);
  await admitHostedMembership(a, [raw, successor], pin());
  expect(a.statements()).toHaveLength(2);
  await expect(admitHostedMembership(a, [raw], pin())).rejects.toThrow();
  expect(a.head?.revision).toBe(2n);
});

it('requires the parsed share-link statement pin, not a transport-supplied newest hash', async () => {
  const a = admission();
  await expect(
    admitHostedMembership(a, [raw, successor], { revision: '1', statement: new Uint8Array(32) }),
  ).rejects.toThrow();
  expect(a.head).toBeNull();
});

it('rejects missing or duplicate revisions before admitting a view', async () => {
  for (const originals of [[], [successor], [raw, raw], [raw, successor, successor]]) {
    const a = admission();
    await expect(admitHostedMembership(a, originals, pin())).rejects.toThrow();
    expect(a.head).toBeNull();
  }
});

it('does not accept a forged owner signature even with a matching unsigned pin', async () => {
  const forged = structuredClone(v.statement);
  forged.signature = c.encodeBinary(new Uint8Array(64));
  const bytes = c.text(JSON.stringify(forged));
  const fakeHash = await c.statement.Envelope.fromJson(bytes).hash();
  const a = admission();
  await expect(
    admitHostedMembership(a, [bytes], { revision: '1', statement: fakeHash }),
  ).rejects.toThrow();
  expect(a.head).toBeNull();
});

it('refuses a root which does not derive the linked space', async () => {
  const a = admission(new Uint8Array(32));
  await expect(admitHostedMembership(a, [raw], pin())).rejects.toThrow();
  expect(a.head).toBeNull();
});

it('keeps the fixed typed refusal for an unavailable or unverifiable baseline', () => {
  expect(new HostedReshareRequired()).toMatchObject({
    code: 'COLAB_HOSTED_RESHARE_REQUIRED',
    message: 'ask the owner to share it again',
  });
});
