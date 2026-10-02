import assert from 'node:assert/strict';
import {
  createHmac,
  createPublicKey,
  generateKeyPairSync,
  randomBytes,
  sign,
  verify,
  type KeyObject,
} from 'node:crypto';
import { test } from 'vitest';
import {
  base64url,
  enrollmentPossessionSigningBytes,
  enrollmentSigningBytes,
  envelopeSigningBytes,
  extCertSigningBytes,
  pairingCode,
  responseKeyInput,
  serverProofInput,
  type Envelope,
} from '../src/canonical-bytes.js';
import {
  DeviceKey,
  certify,
  openSession,
  pair,
  parseLink,
  type Descriptor,
} from '../src/device.js';

/**
 * A stand-in door built on node:crypto, independent of the SDK's WebCrypto
 * path: it checks the enrollment MAC and possession signature, answers 202
 * once, then issues a receipt with serverProof and signs session responses.
 */
const ORIGIN = 'http://127.0.0.1:43210';
const b64 = (bytes: Uint8Array): string => Buffer.from(bytes).toString('base64url');
const raw = (text: string): Buffer => Buffer.from(text, 'base64url');
function publicKeyObject(key: Uint8Array): KeyObject {
  const spki = Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key]);
  return createPublicKey({ key: spki, format: 'der', type: 'spki' });
}
class Door {
  code = randomBytes(16);
  machine = generateKeyPairSync('ed25519');
  machinePublic = this.machine.publicKey.export({ format: 'der', type: 'spki' }).subarray(12);
  descriptor: Descriptor = {
    profile: 'local-v1',
    binding: 'loopback-http',
    machineId: crypto.randomUUID(),
    windowId: crypto.randomUUID(),
    offerId: crypto.randomUUID(),
    address: `${ORIGIN}/r/${'a'.repeat(32)}`,
    serverChallenge: randomBytes(16).toString('hex'),
  };
  clientId = crypto.randomUUID();
  pending = 1;
  tamper: { proof?: boolean; receiptKey?: boolean; signature?: boolean; correlation?: boolean } =
    {};
  link(): string {
    const symbols = base32(this.code);
    return `${ORIGIN}/pair/${b64(Buffer.from(JSON.stringify(this.descriptor)))}#${symbols}`;
  }
  fetch = (async (url: string, init: RequestInit): Promise<Response> => {
    const body = JSON.parse(init.body as string) as Record<string, string>;
    return url.endsWith('/pair') ? this.pair(body) : this.session(body);
  }) as typeof fetch;
  pair(body: Record<string, string>): Response {
    const enrollment = enrollmentSigningBytes({
      profile: 'local-v1',
      machineId: body.machineId!,
      windowId: body.windowId!,
      offerId: body.offerId!,
      serverChallenge: Buffer.from(body.serverChallenge!, 'hex'),
      clientNonce: Buffer.from(body.clientNonce!, 'hex'),
      kind: body.kind as 'browser',
      origin: body.origin!,
      name: body.name!,
      publicKey: raw(body.publicKey!),
    });
    const mac = createHmac('sha256', this.code).update(enrollment).digest();
    assert.equal(body.mac, b64(mac));
    assert.ok(
      verify(
        null,
        enrollmentPossessionSigningBytes(enrollment, mac),
        publicKeyObject(raw(body.publicKey!)),
        raw(body.signature!),
      ),
    );
    if (this.pending-- > 0) return Response.json({ state: 'pending' }, { status: 202 });
    const receipt = Buffer.from(
      JSON.stringify({
        grant: {
          clientId: this.clientId,
          machineId: this.descriptor.machineId,
          profile: 'local-v1',
          publicKey: this.tamper.receiptKey ? b64(randomBytes(32)) : body.publicKey,
          kind: body.kind,
          origin: body.origin,
          name: body.name,
          agents: 'all',
          scopes: ['agents.read'],
          mode: 'direct',
          issuedAtMs: 1,
          expiresAtMs: null,
          revision: 1,
          disabled: false,
        },
        machinePublicKey: b64(this.machinePublic),
      }),
    );
    const responseKey = createHmac('sha256', this.code)
      .update(responseKeyInput(enrollment))
      .digest();
    const proof = createHmac('sha256', responseKey).update(serverProofInput(receipt)).digest();
    if (this.tamper.proof) proof[0]! ^= 1;
    return Response.json({ receipt: b64(receipt), serverProof: b64(proof) });
  }
  async session(body: Record<string, string | number | null>): Promise<Response> {
    const payload = raw(body.payload as string);
    const signed = await envelopeSigningBytes({
      ...(body as unknown as Envelope),
      payload,
    });
    assert.ok(
      verify(null, signed, publicKeyObject(this.devicePublic!), raw(body.signature as string)),
    );
    const sessionId = crypto.randomUUID();
    const reply = Buffer.from(
      JSON.stringify({ sessionId, serverTimeMs: 2, grantRevision: 1, expiresAtMs: null }),
    );
    const response = {
      version: 1 as const,
      profile: 'local-v1' as const,
      kind: 'response' as const,
      id: crypto.randomUUID(),
      correlationId: this.tamper.correlation ? crypto.randomUUID() : (body.id as string),
      machineId: this.descriptor.machineId,
      windowId: this.descriptor.windowId,
      clientId: this.clientId,
      sessionId,
      sequence: '1',
      timestampMs: 3,
      origin: ORIGIN,
      operation: 'session.open',
    };
    const signature = sign(
      null,
      await envelopeSigningBytes({ ...response, payload: reply }),
      this.machine.privateKey,
    );
    if (this.tamper.signature) signature[0]! ^= 1;
    return Response.json({ ...response, payload: b64(reply), signature: b64(signature) });
  }
  devicePublic?: Uint8Array;
}
function base32(code: Uint8Array): string {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  let bits = '';
  for (const byte of code) bits += byte.toString(2).padStart(8, '0');
  return (bits.match(/.{1,5}/g) ?? []).map((g) => alphabet[parseInt(g.padEnd(5, '0'), 2)]).join('');
}
async function paired(door: Door) {
  const key = await DeviceKey.generate();
  door.devicePublic = key.publicKey();
  const { descriptor, code } = parseLink(door.link());
  const result = await pair({
    descriptor,
    code,
    key,
    kind: 'browser',
    origin: ORIGIN,
    name: 'Laptop',
    fetch: door.fetch,
  });
  return { key, result };
}

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

test('an extension certificate verifies over the contract bytes', async () => {
  const key = await DeviceKey.generate();
  const extensionKey = randomBytes(32);
  const certificate = await certify(
    key,
    { extension: 'colab', purpose: 'sign', publicKey: extensionKey },
    1790770000000,
  );
  assert.equal(certificate.publicKey, base64url(extensionKey));
  const bytes = extCertSigningBytes({
    extension: 'colab',
    purpose: 'sign',
    publicKey: extensionKey,
    issuedAtMs: 1790770000000,
  });
  assert.ok(verify(null, bytes, publicKeyObject(key.publicKey()), raw(certificate.signature)));
  await assert.rejects(
    certify(key, { extension: 'Colab', purpose: 'sign', publicKey: extensionKey }),
  );
});

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
