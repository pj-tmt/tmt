import assert from 'node:assert/strict';
import {
  createHash,
  createHmac,
  createPrivateKey,
  createPublicKey,
  generateKeyPairSync,
  randomBytes,
  randomUUID,
  sign,
  timingSafeEqual,
  verify,
  type KeyObject,
} from 'node:crypto';
import fs from 'node:fs';

// Test-only protocol peer. Expected bytes come from committed independent
// Python/WebCrypto vectors; no Remote SDK or runtime code is imported.
export interface RemoteEnvelope {
  version: number;
  profile: string;
  kind: string;
  id: string;
  correlationId: string | null;
  machineId: string;
  windowId: string;
  clientId: string;
  sessionId: string;
  sequence: string;
  timestampMs: number;
  origin: string;
  operation: string;
  payload: string;
  signature: string;
}
export interface RemoteDescriptor {
  profile: string;
  machineId: string;
  windowId: string;
  offerId: string;
  serverChallenge: string;
  address: string;
}
export interface PairedDevice {
  clientId: string;
  machineId: string;
  machinePublicKey: string;
  revision: number;
}
export function object(value: unknown): Record<string, unknown> {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value));
  return value as Record<string, unknown>;
}
export function text(value: unknown): string {
  assert.equal(typeof value, 'string');
  return value as string;
}
export function decode64(value: string, length?: number): Buffer {
  assert.match(value, /^[A-Za-z0-9_-]*$/);
  const bytes = Buffer.from(value, 'base64url');
  assert.equal(bytes.toString('base64url'), value, 'canonical unpadded base64url');
  if (length !== undefined) assert.equal(bytes.length, length);
  return bytes;
}
function lp(bytes: Uint8Array): Buffer {
  const count = Buffer.alloc(4);
  count.writeUInt32BE(bytes.length);
  return Buffer.concat([count, bytes]);
}
function fields(values: Array<string | Uint8Array>): Buffer {
  return Buffer.concat(values.map((value) => lp(typeof value === 'string' ? utf8(value) : value)));
}
function utf8(value: string): Buffer {
  assert.equal(
    Buffer.from(value, 'utf8').toString('utf8'),
    value,
    'no unpaired Unicode surrogates'
  );
  return Buffer.from(value, 'utf8');
}
function sha256(bytes: Uint8Array): Buffer {
  return createHash('sha256').update(bytes).digest();
}
function hmac(key: Uint8Array, bytes: Uint8Array): Buffer {
  return createHmac('sha256', key).update(bytes).digest();
}
export function envelopeBytes(envelope: Omit<RemoteEnvelope, 'signature'>): Buffer {
  return fields([
    'tmt-message-v1',
    String(envelope.version),
    envelope.profile,
    envelope.kind,
    envelope.id,
    envelope.correlationId ?? '',
    envelope.machineId,
    envelope.windowId,
    envelope.clientId,
    envelope.sessionId,
    envelope.sequence,
    String(envelope.timestampMs),
    envelope.origin,
    envelope.operation,
    sha256(decode64(envelope.payload)),
  ]);
}
function publicKey(bytes: Buffer): KeyObject {
  assert.equal(bytes.length, 32);
  return createPublicKey({
    key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), bytes]),
    format: 'der',
    type: 'spki',
  });
}
function codeBytes(code: string): Buffer {
  const symbols = code.replace(/[ -]/g, '');
  assert.equal(symbols.length, 26);
  let bits = '';
  for (const symbol of symbols) {
    const index = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'.indexOf(symbol);
    assert(index >= 0);
    bits += index.toString(2).padStart(5, '0');
  }
  assert.equal(bits.slice(128), '00');
  return Buffer.from(
    Array.from({ length: 16 }, (_, i) => parseInt(bits.slice(i * 8, i * 8 + 8), 2))
  );
}
function enrollmentBytes(input: Record<string, unknown>): Buffer {
  return fields([
    'tmt-device-pair-v1',
    text(input.profile),
    text(input.machineId),
    text(input.windowId),
    text(input.offerId),
    Buffer.from(text(input.serverChallenge), 'hex'),
    Buffer.from(text(input.clientNonce), 'hex'),
    text(input.kind),
    text(input.origin),
    text(input.name),
    decode64(text(input.publicKey), 32),
  ]);
}
function responseKey(code: Buffer, enrollment: Buffer): Buffer {
  return hmac(code, fields(['tmt-device-pair-response-key-v1', enrollment]));
}
function serverProof(key: Buffer, receipt: Buffer): Buffer {
  return hmac(key, fields(['tmt-device-pair-response-v1', receipt]));
}
export class RemoteDevice {
  readonly name = 'E2E owner device';
  readonly origin = 'cli';
  readonly publicBytes: Buffer;
  private readonly key: KeyObject;
  constructor() {
    const keys = generateKeyPairSync('ed25519');
    this.key = keys.privateKey;
    this.publicBytes = keys.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32);
  }
  envelope(input: Omit<RemoteEnvelope, 'signature'>): RemoteEnvelope {
    return {
      ...input,
      signature: sign(null, envelopeBytes(input), this.key).toString('base64url'),
    };
  }
  enrollment(
    descriptor: RemoteDescriptor,
    code: string
  ): {
    body: Record<string, unknown>;
    accept: (response: unknown) => PairedDevice;
  } {
    const input = {
      profile: descriptor.profile,
      machineId: descriptor.machineId,
      windowId: descriptor.windowId,
      offerId: descriptor.offerId,
      serverChallenge: descriptor.serverChallenge,
      clientNonce: randomBytes(16).toString('hex'),
      kind: 'cli',
      origin: this.origin,
      name: this.name,
      publicKey: this.publicBytes.toString('base64url'),
    };
    const bytes = enrollmentBytes(input);
    const codeKey = codeBytes(code);
    const mac = hmac(codeKey, bytes);
    const proof = sign(null, fields(['tmt-device-pair-possession-v1', bytes, mac]), this.key);
    return {
      body: { ...input, mac: mac.toString('base64url'), signature: proof.toString('base64url') },
      accept: (response) => {
        const reply = object(response);
        const receiptBytes = decode64(text(reply.receipt));
        const expected = serverProof(responseKey(codeKey, bytes), receiptBytes);
        assert(
          timingSafeEqual(expected, decode64(text(reply.serverProof), 32)),
          'pair receipt proof'
        );
        const receipt = object(JSON.parse(receiptBytes.toString('utf8')));
        const grant = object(receipt.grant);
        for (const field of [
          'profile',
          'machineId',
          'kind',
          'origin',
          'name',
          'publicKey',
        ] as const)
          assert.equal(grant[field], input[field]);
        assert.equal(grant.disabled, false);
        assert.equal(grant.revision, 1);
        assert.equal(grant.agents, 'all');
        assert.equal(grant.mode, 'direct');
        assert.equal(grant.expiresAtMs, null);
        const machinePublicKey = text(receipt.machinePublicKey);
        decode64(machinePublicKey, 32);
        return {
          clientId: text(grant.clientId),
          machineId: descriptor.machineId,
          machinePublicKey,
          revision: 1,
        };
      },
    };
  }
}
export function requestEnvelope(
  device: RemoteDevice,
  paired: PairedDevice,
  windowId: string,
  sessionId: string,
  sequence: string,
  operation: string,
  input: unknown,
  options: { id?: string; kind?: 'request' | 'control'; payload?: Buffer } = {}
): RemoteEnvelope {
  return device.envelope({
    version: 1,
    profile: 'local-v1',
    kind: options.kind ?? 'request',
    id: options.id ?? randomUUID(),
    correlationId: null,
    machineId: paired.machineId,
    windowId,
    clientId: paired.clientId,
    sessionId,
    sequence,
    timestampMs: Date.now(),
    origin: device.origin,
    operation,
    payload: (options.payload ?? utf8(JSON.stringify(input))).toString('base64url'),
  });
}
export function verifyResponse(
  reply: unknown,
  request: RemoteEnvelope,
  paired: PairedDevice
): {
  envelope: RemoteEnvelope;
  payload: Record<string, unknown>;
} {
  const input = object(reply);
  for (const field of ['profile', 'machineId', 'windowId', 'clientId', 'origin', 'operation'])
    assert.equal(input[field], object(request)[field], `signed response ${field}`);
  assert.equal(input.version, 1);
  assert.equal(input.kind, 'response');
  assert.equal(input.correlationId, request.id);
  for (const field of ['id', 'sessionId', 'sequence', 'payload', 'signature']) text(input[field]);
  assert.match(
    text(input.id),
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/
  );
  assert(Number.isSafeInteger(input.timestampMs));
  assert.match(text(input.sequence), /^[1-9][0-9]*$/);
  const envelope = input as unknown as RemoteEnvelope;
  assert(
    verify(
      null,
      envelopeBytes(envelope),
      publicKey(decode64(paired.machinePublicKey, 32)),
      decode64(envelope.signature, 64)
    ),
    'pinned machine signature'
  );
  if (request.operation !== 'session.open') assert.equal(envelope.sessionId, request.sessionId);
  return { envelope, payload: object(JSON.parse(decode64(envelope.payload).toString('utf8'))) };
}
/** Independent fixture checks are a prerequisite for the real wire assertions. */
export function assertRemoteDeviceVectors(): void {
  const read = (relative: string) =>
    JSON.parse(fs.readFileSync(new URL(relative, import.meta.url), 'utf8'));
  const vectors = read('../../../extensions/tmt-remote/typescript/remote-client/test/vectors.json');
  const signatures = read(
    '../../../extensions/tmt-remote/rust/tmt-remote/tests/fixtures/webcrypto-vectors.json'
  );
  const seed = '9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60';
  const key = createPrivateKey({
    key: Buffer.from(`302e020100300506032b657004220420${seed}`, 'hex'),
    format: 'der',
    type: 'pkcs8',
  });
  for (const vector of vectors.envelopes) {
    const input = {
      ...vector.input,
      payload: Buffer.from(vector.input.payload, 'hex').toString('base64url'),
    };
    const bytes = envelopeBytes(input);
    assert.equal(bytes.toString('hex'), vector.hex);
    assert.equal(sha256(bytes).toString('hex'), vector.sha256);
  }
  for (const vector of vectors.enrollments) {
    const input = {
      ...vector.input,
      publicKey: Buffer.from(vector.input.publicKey, 'hex').toString('base64url'),
    };
    assert.equal(enrollmentBytes(input).toString('hex'), vector.hex);
  }
  for (const vector of signatures.cases) {
    const bytes = Buffer.from(vector.message, 'hex');
    const signature = sign(null, bytes, key);
    assert.equal(signature.toString('hex'), vector.signature);
    assert(verify(null, bytes, createPublicKey(key), signature));
    const changed = Buffer.concat([bytes, Buffer.from([0])]);
    assert.equal(verify(null, changed, createPublicKey(key), signature), false);
  }
  const mac = read(
    '../../../extensions/tmt-remote/rust/tmt-remote/tests/fixtures/mac-vectors.json'
  );
  const code = Buffer.from(mac.code, 'hex'),
    enrollment = Buffer.from(mac.enrollment, 'hex');
  assert.equal(hmac(code, enrollment).toString('hex'), mac.enrollmentMac);
  const keyBytes = responseKey(code, enrollment);
  assert.equal(keyBytes.toString('hex'), mac.responseKey);
  assert.equal(
    serverProof(keyBytes, Buffer.from(mac.receipt, 'hex')).toString('hex'),
    mac.serverProof
  );
  for (const vector of vectors.pairingCodes)
    assert.equal(codeBytes(vector.text).toString('hex'), vector.code);
}
