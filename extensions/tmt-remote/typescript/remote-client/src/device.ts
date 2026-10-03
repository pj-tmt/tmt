import { registerChannel, verifyEd25519, verifyResponse } from './session-channel.js';
import {
  base64url,
  base64urlBytes,
  enrollmentPossessionSigningBytes,
  enrollmentSigningBytes,
  envelopeSigningBytes,
  extCertSigningBytes,
  pairingCode,
  responseKeyInput,
  serverProofInput,
  type ExtCert,
} from './canonical-bytes.js';

/**
 * Device-side SDK for remote-channel-v1: the device key, the pairing ceremony,
 * `session.open` and `tmt-ext-cert-v1`. Network access goes through an injected
 * fetch; key persistence is the caller's (the browser page stores the opaque
 * CryptoKey in IndexedDB by structured clone).
 */

function requireValue(condition: boolean, label: string): asserts condition {
  if (!condition) throw new Error(`Invalid ${label}.`);
}
/** A fresh ArrayBuffer-backed copy, as WebCrypto requires. */
const owned = (bytes: Uint8Array): Uint8Array<ArrayBuffer> => new Uint8Array(bytes);
const utf8 = new TextEncoder();
const strictUtf8 = new TextDecoder('utf-8', { fatal: true });
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const HEX16 = /^[0-9a-f]{32}$/;

function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}
function fromHex(text: string): Uint8Array {
  requireValue(HEX16.test(text), 'hex');
  return Uint8Array.from(text.match(/../g)!, (pair) => parseInt(pair, 16));
}
function random(length: number): Uint8Array {
  return crypto.getRandomValues(new Uint8Array(length));
}
/** Strict base64url of unknown length: the decoded length follows from the text. */
function decode(text: string): Uint8Array {
  return base64urlBytes(text, Math.floor((text.length * 6) / 8));
}
async function hmac(key: Uint8Array, message: Uint8Array): Promise<Uint8Array> {
  const native = await crypto.subtle.importKey(
    'raw',
    owned(key),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  return new Uint8Array(await crypto.subtle.sign('HMAC', native, owned(message)));
}
/** Constant-time HMAC check through WebCrypto. */
async function verifyHmac(key: Uint8Array, message: Uint8Array, tag: Uint8Array): Promise<boolean> {
  const native = await crypto.subtle.importKey(
    'raw',
    owned(key),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['verify'],
  );
  return tag.length === 32 && crypto.subtle.verify('HMAC', native, owned(tag), owned(message));
}
/** A device's Ed25519 key. The private half never leaves its non-extractable CryptoKey. */
export class DeviceKey {
  readonly #private: CryptoKey;
  readonly #public: Uint8Array;
  private constructor(privateKey: CryptoKey, publicKey: Uint8Array) {
    this.#private = privateKey;
    this.#public = new Uint8Array(publicKey);
  }
  static async generate(): Promise<DeviceKey> {
    const pair = (await crypto.subtle.generateKey('Ed25519', false, [
      'sign',
      'verify',
    ])) as CryptoKeyPair;
    const publicKey = new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey));
    return new DeviceKey(pair.privateKey, publicKey);
  }
  /** Restore a stored handle; it must be a non-extractable Ed25519 signing key for `publicKey`. */
  static async fromHandle(privateKey: CryptoKey, publicKey: Uint8Array): Promise<DeviceKey> {
    requireValue(
      privateKey.type === 'private' &&
        !privateKey.extractable &&
        privateKey.algorithm.name === 'Ed25519' &&
        privateKey.usages.includes('sign') &&
        publicKey.length === 32,
      'device key handle',
    );
    const key = new DeviceKey(privateKey, publicKey);
    const probe = utf8.encode('tmt-device-key-probe');
    requireValue(await verifyEd25519(publicKey, probe, await key.sign(probe)), 'device key pair');
    return key;
  }
  publicKey(): Uint8Array {
    return new Uint8Array(this.#public);
  }
  /** The opaque handle for structured-clone storage; never exported. */
  handle(): CryptoKey {
    return this.#private;
  }
  async sign(message: Uint8Array): Promise<Uint8Array> {
    return new Uint8Array(await crypto.subtle.sign('Ed25519', this.#private, owned(message)));
  }
}

/** The pairing descriptor printed by `tmt remote pair`, carried in the link path. */
export interface Descriptor {
  profile: 'local-v1';
  binding: 'loopback-http';
  machineId: string;
  windowId: string;
  offerId: string;
  /** Door origin plus the `/r/<prefix>` route prefix. */
  address: string;
  serverChallenge: string;
}
const DESCRIPTOR_FIELDS = [
  'profile',
  'binding',
  'machineId',
  'windowId',
  'offerId',
  'address',
  'serverChallenge',
];
/** Parse `<door>/pair/<descriptor>#<code>`. The page removes the fragment before calling this. */
export function parseLink(link: string): { descriptor: Descriptor; code: Uint8Array } {
  const url = new URL(link);
  const encoded = /^\/pair\/([A-Za-z0-9_-]+)$/.exec(url.pathname)?.[1];
  requireValue(encoded !== undefined && url.search === '', 'pairing link');
  const descriptor = JSON.parse(strictUtf8.decode(decode(encoded))) as Record<string, unknown>;
  requireValue(
    Object.keys(descriptor).length === DESCRIPTOR_FIELDS.length &&
      DESCRIPTOR_FIELDS.every((field) => typeof descriptor[field] === 'string') &&
      descriptor.profile === 'local-v1' &&
      descriptor.binding === 'loopback-http' &&
      ['machineId', 'windowId', 'offerId'].every((id) => UUID.test(descriptor[id] as string)) &&
      HEX16.test(descriptor.serverChallenge as string) &&
      (descriptor.address as string).startsWith(`${url.origin}/r/`),
    'pairing descriptor',
  );
  return { descriptor: descriptor as unknown as Descriptor, code: pairingCode(url.hash.slice(1)) };
}

/** What the device keeps after a verified pairing. */
export interface Paired {
  clientId: string;
  machineId: string;
  machinePublicKey: Uint8Array;
  kind: 'browser' | 'cli';
  origin: string;
  /** Door origin plus route prefix, for `session.open`. */
  address: string;
  grantRevision: number;
}
export interface PairOptions {
  descriptor: Descriptor;
  code: Uint8Array;
  key: DeviceKey;
  kind: 'browser' | 'cli';
  origin: string;
  name: string;
  fetch?: typeof fetch;
  /** Bound on retrying a pending candidate; defaults to the ten-minute offer lifetime. */
  deadlineMs?: number;
}
/**
 * Submit one enrollment candidate and retry it exactly while the owner has
 * not answered. The machine key and grant are accepted only after
 * `serverProof` verifies over the exact receipt bytes.
 */
export async function pair(options: PairOptions): Promise<Paired> {
  const { descriptor, code, key } = options;
  const send = options.fetch ?? fetch;
  const publicKey = key.publicKey();
  const clientNonce = random(16);
  const enrollment = enrollmentSigningBytes({
    profile: 'local-v1',
    machineId: descriptor.machineId,
    windowId: descriptor.windowId,
    offerId: descriptor.offerId,
    serverChallenge: fromHex(descriptor.serverChallenge),
    clientNonce,
    kind: options.kind,
    origin: options.origin,
    name: options.name,
    publicKey,
  });
  const mac = await hmac(code, enrollment);
  const signature = await key.sign(enrollmentPossessionSigningBytes(enrollment, mac));
  const body = JSON.stringify({
    profile: 'local-v1',
    machineId: descriptor.machineId,
    windowId: descriptor.windowId,
    offerId: descriptor.offerId,
    serverChallenge: descriptor.serverChallenge,
    clientNonce: hex(clientNonce),
    kind: options.kind,
    origin: options.origin,
    name: options.name,
    publicKey: base64url(publicKey),
    mac: base64url(mac),
    signature: base64url(signature),
  });
  const deadline = Date.now() + (options.deadlineMs ?? 600_000);
  for (;;) {
    const response = await send(`${descriptor.address}/pair`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body,
    });
    if (response.status === 202 && Date.now() < deadline) continue;
    if (response.status !== 200) throw new Error('Pairing was refused or ended.');
    const reply = (await response.json()) as { receipt?: unknown; serverProof?: unknown };
    requireValue(
      typeof reply.receipt === 'string' && typeof reply.serverProof === 'string',
      'pairing reply',
    );
    const receipt = decode(reply.receipt);
    const responseKey = await hmac(code, responseKeyInput(enrollment));
    requireValue(
      await verifyHmac(responseKey, serverProofInput(receipt), decode(reply.serverProof)),
      'server proof',
    );
    return accept(receipt, descriptor, options, publicKey);
  }
}
/** Check the proven receipt names this machine, this device and a well-formed grant. */
function accept(
  receipt: Uint8Array,
  descriptor: Descriptor,
  options: PairOptions,
  publicKey: Uint8Array,
): Paired {
  const parsed = JSON.parse(strictUtf8.decode(receipt)) as {
    grant?: Record<string, unknown>;
    machinePublicKey?: unknown;
  };
  const grant = parsed.grant ?? {};
  requireValue(
    typeof parsed.machinePublicKey === 'string' &&
      typeof grant.clientId === 'string' &&
      UUID.test(grant.clientId) &&
      grant.machineId === descriptor.machineId &&
      grant.profile === 'local-v1' &&
      grant.publicKey === base64url(publicKey) &&
      grant.kind === options.kind &&
      grant.origin === options.origin &&
      Number.isSafeInteger(grant.revision) &&
      grant.disabled === false,
    'receipt',
  );
  return {
    clientId: grant.clientId,
    machineId: descriptor.machineId,
    machinePublicKey: base64urlBytes(parsed.machinePublicKey, 32),
    kind: options.kind,
    origin: options.origin,
    address: descriptor.address,
    grantRevision: grant.revision as number,
  };
}

/** A verified `session.open` response payload. */
export interface Session {
  sessionId: string;
  serverTimeMs: number;
  grantRevision: number;
  expiresAtMs: number | null;
}
/**
 * Open the device's session for the current remote run. `windowId` names that
 * run; a browser on the door receives its session cookie with the response.
 */
export async function openSession(
  paired: Paired,
  key: DeviceKey,
  windowId: string,
  send: typeof fetch = fetch,
): Promise<Session> {
  const id = crypto.randomUUID();
  const payload = utf8.encode(JSON.stringify({ clientNonce: hex(random(16)) }));
  const control = {
    kind: 'control' as const,
    id,
    correlationId: null,
    machineId: paired.machineId,
    windowId,
    clientId: paired.clientId,
    sessionId: 'new',
    sequence: '0',
    timestampMs: Date.now(),
    origin: paired.origin,
    operation: 'session.open',
  };
  const signature = await key.sign(
    await envelopeSigningBytes({ version: 1, profile: 'local-v1', ...control, payload }),
  );
  const response = await send(`${paired.address}/append`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    credentials: 'same-origin',
    body: JSON.stringify({
      version: 1,
      profile: 'local-v1',
      ...control,
      payload: base64url(payload),
      signature: base64url(signature),
    }),
  });
  if (response.status !== 200) throw new Error('The session was refused.');
  const reply = await verifyResponse(await response.json(), paired, {
    id,
    windowId,
    operation: 'session.open',
    after: 0n,
  });
  requireValue(reply.sequence === 1n, 'session response sequence');
  const session = JSON.parse(strictUtf8.decode(reply.payload)) as Session;
  requireValue(
    session.sessionId === reply.sessionId &&
      Number.isSafeInteger(session.serverTimeMs) &&
      session.serverTimeMs >= 0 &&
      Number.isSafeInteger(session.grantRevision) &&
      session.grantRevision > 0 &&
      (session.expiresAtMs === null ||
        (Number.isSafeInteger(session.expiresAtMs) && session.expiresAtMs >= 0)),
    'session payload',
  );
  registerChannel(session, paired, key, windowId, send);
  return session;
}

/** A device-signed `tmt-ext-cert-v1` certificate. */
export interface ExtCertificate {
  extension: string;
  purpose: 'sign' | 'enc';
  publicKey: string;
  issuedAtMs: number;
  signature: string;
}
/** Certify an extension key with the device key. Callers fix `extension` from the door's mount. */
export async function certify(
  key: DeviceKey,
  value: Omit<ExtCert, 'issuedAtMs'>,
  issuedAtMs = Date.now(),
): Promise<ExtCertificate> {
  const signature = await key.sign(extCertSigningBytes({ ...value, issuedAtMs }));
  return {
    extension: value.extension,
    purpose: value.purpose,
    publicKey: base64url(value.publicKey),
    issuedAtMs,
    signature: base64url(signature),
  };
}
