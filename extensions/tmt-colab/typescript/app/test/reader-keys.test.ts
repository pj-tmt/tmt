import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import { deriveReaderKeys } from '../src/reader-keys.js';
import type { ReaderLink } from '../src/reader-link.js';

const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (value: string) =>
  Uint8Array.from(value.match(/../g) ?? [], (byte) => parseInt(byte, 16));
const target = (): ReaderLink => ({
  space: v.space,
  page: v.page,
  link: v.linkId,
  revision: '1',
  statement: hex(v.statementHash),
  seed: hex(v.linkSeed),
  root: hex(v.public),
  role: 'viewer',
});

it('the door reader retains the independent link/device/token/chain derivation', async () => {
  const input = target(),
    keys = await deriveReaderKeys(input, 'door');
  expect(input.seed).toEqual(new Uint8Array(32));
  expect(keys.link.rulesToken).toBe(v.linkRulesToken);
  expect(keys.link.signingPublic).toEqual(hex(v.linkSigningPublic));
  expect(keys.device.id).toBe(v.linkDevice.id);
  expect(JSON.parse(c.decodeText(keys.chain))).toEqual(v.linkDevice.chain);
  expect(await c.certificate.Chain.fromJson(keys.chain).digest()).toEqual(
    hex(v.linkDevice.chainDigest),
  );
});

it('the hosted reader uses its distinct reviewed device and chain, with the same link token', async () => {
  const input = target(),
    keys = await deriveReaderKeys(input, 'hosted');
  expect(input.seed).toEqual(new Uint8Array(32));
  expect(keys.link.rulesToken).toBe(v.linkRulesToken);
  expect(keys.device.id).toBe(v.linkHostedDevice.id);
  expect(keys.device.id).not.toBe(v.linkDevice.id);
  expect(JSON.parse(c.decodeText(keys.chain))).toEqual(v.linkHostedDevice.chain);
  expect(await c.certificate.Chain.fromJson(keys.chain).digest()).toEqual(
    hex(v.linkHostedDevice.chainDigest),
  );
});

it('the seed is consumed even when the root or derived-device input is refused', async () => {
  for (const change of [{ root: new Uint8Array(32) }, { revision: '0' }]) {
    const input = { ...target(), ...change };
    await expect(deriveReaderKeys(input, 'door')).rejects.toThrow();
    expect(input.seed).toEqual(new Uint8Array(32));
  }
});

it('a second tab produces byte-identical proof without keeping either bearer seed', async () => {
  const one = target(),
    two = target();
  const first = await deriveReaderKeys(one, 'door'),
    second = await deriveReaderKeys(two, 'door');
  expect(first.chain).toEqual(second.chain);
  expect(first.device.id).toBe(second.device.id);
  expect(one.seed.some(Boolean)).toBe(false);
  expect(two.seed.some(Boolean)).toBe(false);
});
