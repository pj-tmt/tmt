import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import * as c from '../src/index.js';
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (s: string): c.Bytes => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));

describe('link keys', () => {
  it('derive the vector link signing and encryption publics from the seed', async () => {
    const keys = await c.link.deriveLink(hex(v.linkSeed), v.space, v.linkId);
    expect(keys.signingPublic).toEqual(hex(v.linkSigningPublic));
    expect(keys.encryption.publicKey()).toEqual(hex(v.linkEncryptionPublic));
    // Another space or link ID is another key, never an alias.
    const other = await c.link.deriveLink(
      hex(v.linkSeed),
      v.space,
      '00000000-0000-4000-8000-000000000061',
    );
    expect(other.signingPublic).not.toEqual(keys.signingPublic);
  });
  it('rejects a seed that is not 32 bytes', async () => {
    await expect(c.link.deriveLink(new Uint8Array(31), v.space, v.linkId)).rejects.toThrow();
  });
});

describe('link-device chain', () => {
  const statement = hex('11'.repeat(32));
  async function chain(overrides: Partial<c.link.LinkDevice> = {}) {
    const keys = await c.link.deriveLink(hex(v.linkSeed), v.space, v.linkId);
    const pair = (await crypto.subtle.generateKey('Ed25519', false, [
      'sign',
      'verify',
    ])) as CryptoKeyPair;
    const device = {
      deviceId: '30000000-0000-4000-8000-000000000001',
      signingPublic: new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey)),
      encryptionPublic: hex('22'.repeat(32)),
      membershipRevision: '4',
      issuerStatement: statement,
      issuedAt: 1000,
      expiresAt: 2000,
      ...overrides,
    };
    return { keys, device, json: await c.link.certifyDevice(keys, device) };
  }
  it('is verified by the link key against the introducing statement and nothing else', async () => {
    const { keys, device, json } = await chain();
    const parsed = c.certificate.Chain.fromJson(json);
    const cert = parsed.certificate();
    expect(cert).toMatchObject({
      space: v.space,
      issuerKind: 'link',
      issuerId: v.linkId,
      deviceId: device.deviceId,
      membershipRevision: '4',
    });
    await parsed.verify(statement, cert, keys.signingPublic);
    await expect(parsed.verify(hex('12'.repeat(32)), cert, keys.signingPublic)).rejects.toThrow();
    const wrong = await c.link.deriveLink(hex('00'.repeat(32)), v.space, v.linkId);
    await expect(parsed.verify(statement, cert, wrong.signingPublic)).rejects.toThrow();
  });
  it('refuses malformed device fields before signing', async () => {
    await expect(chain({ membershipRevision: '0' })).rejects.toThrow();
    await expect(chain({ issuerStatement: new Uint8Array(31) })).rejects.toThrow();
    await expect(chain({ expiresAt: 1000 })).rejects.toThrow();
  });
  it('builds the exact reader session input', () => {
    const input = c.link.readerSessionInput({
      challengeId: '40000000-0000-4000-8000-000000000001',
      nonce: hex('33'.repeat(32)),
      space: v.space,
      page: v.page,
      epoch: '1',
      chainDigest: hex('44'.repeat(32)),
      expiresAt: 61000,
    });
    expect(c.decodeText(c.fields(input, 9, 4096)[0])).toBe('tmt-colab-reader-session-v1');
    expect(c.decodeText(c.fields(input, 9, 4096)[8])).toBe('61000');
  });
});
