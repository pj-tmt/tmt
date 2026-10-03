import assert from 'node:assert/strict';
import { verify } from 'node:crypto';
import { Door, paired, raw, publicKeyObject, ORIGIN, b64, base32 } from './door.js';
import { test } from 'vite-plus/test';
import vectors from './vectors.json' with { type: 'json' };
import signatures from '../../../rust/tmt-remote/tests/fixtures/webcrypto-vectors.json' with { type: 'json' };
import {
  base64url,
  extCertSigningBytes,
  pairingCode,
  type ExtCert,
} from '../src/canonical-bytes.js';
import { DeviceKey, certify, openSession, parseLink } from '../src/device.js';

test('pairing retries a pending candidate and accepts only a proven receipt', async () => {
  const door = new Door();
  const { result } = await paired(door);
  assert.equal(door.pending, -1, 'one pending answer, then the receipt');
  assert.equal(result.clientId, door.clientId);
  assert.deepEqual(result.machinePublicKey, new Uint8Array(door.machinePublic));
  assert.equal(result.address, door.descriptor.address);
  for (const tamper of [{ proof: true }, { receiptKey: true }]) {
    const bad = new Door();
    bad.tamper = tamper;
    await assert.rejects(paired(bad), JSON.stringify(tamper));
  }
});

test('session.open is signed by the device and its response by the machine', async () => {
  const door = new Door();
  const { key, result } = await paired(door);
  const session = await openSession(result, key, door.descriptor.windowId, door.fetch);
  assert.equal(session.grantRevision, 1);
  assert.equal(session.expiresAtMs, null);
  for (const tamper of [{ signature: true }, { correlation: true }]) {
    door.tamper = tamper;
    await assert.rejects(
      openSession(result, key, door.descriptor.windowId, door.fetch),
      JSON.stringify(tamper),
    );
  }
});

test('the device key is non-extractable and restores only as itself', async () => {
  const key = await DeviceKey.generate();
  assert.equal(key.handle().extractable, false);
  await assert.rejects(crypto.subtle.exportKey('pkcs8', key.handle()));
  const restored = await DeviceKey.fromHandle(key.handle(), key.publicKey());
  assert.deepEqual(restored.publicKey(), key.publicKey());
  const other = await DeviceKey.generate();
  await assert.rejects(DeviceKey.fromHandle(key.handle(), other.publicKey()));
  const exportable = (await crypto.subtle.generateKey('Ed25519', true, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  await assert.rejects(DeviceKey.fromHandle(exportable.privateKey, key.publicKey()));
});

for (const vector of vectors.extCerts) {
  test(`fixed certificate signature binds every field: ${vector.name}`, async () => {
    // Public RFC 8032 TEST 1 seed only, restored as a non-extractable handle.
    const handle = await crypto.subtle.importKey(
      'pkcs8',
      Buffer.from(
        '302e020100300506032b657004220420' +
          '9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60',
        'hex',
      ),
      'Ed25519',
      false,
      ['sign'],
    );
    const devicePublic = Buffer.from(
      'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a',
      'hex',
    );
    const key = await DeviceKey.fromHandle(handle, devicePublic);
    const value = {
      ...vector.input,
      publicKey: Buffer.from(vector.input.publicKey, 'hex'),
    } as ExtCert;
    const fixed = signatures.cases.find((v) => v.name === vector.name);
    assert.ok(fixed, 'every Python certificate has a fixed WebCrypto signature');
    const certificate = await certify(key, value, value.issuedAtMs);
    assert.equal(certificate.publicKey, base64url(value.publicKey));
    assert.equal(raw(certificate.signature).toString('hex'), fixed.signature);
    const baseline = extCertSigningBytes(value);
    assert.equal(Buffer.from(baseline).toString('hex'), fixed.message);
    const signature = raw(certificate.signature);
    const verifies = (message: Uint8Array): boolean =>
      verify(null, message, publicKeyObject(devicePublic), signature);
    assert.ok(verifies(baseline));
    for (const change of [
      { extension: 'other' },
      { purpose: value.purpose === 'sign' ? ('enc' as const) : ('sign' as const) },
      { publicKey: Uint8Array.from(value.publicKey, (byte, i) => (i === 0 ? byte ^ 1 : byte)) },
      { issuedAtMs: value.issuedAtMs === 0 ? 1 : value.issuedAtMs - 1 },
    ]) {
      assert.equal(verifies(extCertSigningBytes({ ...value, ...change })), false);
    }
    const domain = baseline.slice();
    domain[4 + 'tmt-ext-cert-v1'.length - 1] = '2'.charCodeAt(0);
    assert.equal(verifies(domain), false);
    const littleEndian = baseline.slice();
    littleEndian.subarray(0, 4).reverse();
    assert.equal(verifies(littleEndian), false);
    const differentSignature = Buffer.from(signature);
    differentSignature[0]! ^= 1;
    assert.equal(verify(null, baseline, publicKeyObject(devicePublic), differentSignature), false);
    const other = await DeviceKey.generate();
    assert.equal(verify(null, baseline, publicKeyObject(other.publicKey()), signature), false);
    await assert.rejects(certify(key, { ...value, extension: 'Colab' }, value.issuedAtMs));
  });
}

test('pairing links carry a strict descriptor and the code only in the fragment', () => {
  const door = new Door();
  const { descriptor, code } = parseLink(door.link());
  assert.deepEqual(descriptor, door.descriptor);
  assert.deepEqual(code, pairingCode(base32(door.code)));
  const encode = (value: object): string => b64(Buffer.from(JSON.stringify(value)));
  const fragment = `#${base32(door.code)}`;
  for (const [label, link] of [
    ['query', `${ORIGIN}/pair/${encode(door.descriptor)}?x=1${fragment}`],
    ['extra field', `${ORIGIN}/pair/${encode({ ...door.descriptor, extra: 1 })}${fragment}`],
    [
      'other origin address',
      `${ORIGIN}/pair/${encode({ ...door.descriptor, address: 'http://127.0.0.1:1/r/x' })}${fragment}`,
    ],
    ['no code', `${ORIGIN}/pair/${encode(door.descriptor)}`],
    ['other path', `${ORIGIN}/x/pair/${encode(door.descriptor)}${fragment}`],
  ]) {
    assert.throws(() => parseLink(link!), label);
  }
});
