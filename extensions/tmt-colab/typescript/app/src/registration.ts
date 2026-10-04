import {
  binary,
  certificate,
  decimal,
  deriveSpaceId,
  equal,
  exactKeys,
  generatedId,
  requireValue,
  statement,
  strictJson,
  text,
  encodeBinary,
} from '@tmt/colab-client';
import { deviceKeys, type DeviceKeys } from './keyring.js';

interface CertifiedKey {
  publicKey: string;
  issuedAtMs: number;
  signature: string;
}
export interface RemoteSdk {
  reopenSession(): Promise<unknown>;
  certifyKey(purpose: 'sign' | 'enc', publicKey: Uint8Array): Promise<CertifiedKey>;
}
export interface Registration {
  remoteSession?: unknown;
  deviceId: string;
  keys: DeviceKeys;
  chain: certificate.Chain;
  /** Absent for a read-only link device, which has no member issuer to verify. */
  issuer?: statement.Envelope;
}
export async function remoteSdk(): Promise<RemoteSdk> {
  const path = '/sdk/remote-v1.js';
  return import(/* @vite-ignore */ path) as Promise<RemoteSdk>;
}
export async function jsonResponse(response: Response, cap: number) {
  if (!response.ok) throw new Error(`Colab registration unavailable (${response.status})`);
  // Stream limits apply even when the server omits or lies about Content-Length.
  const reader = response.body?.getReader();
  if (!reader) throw new Error('Empty Colab response');
  const parts: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > cap) throw new Error('Colab response exceeds limit');
      parts.push(value);
    }
    const raw = new Uint8Array(size);
    let offset = 0;
    for (const part of parts) {
      raw.set(part, offset);
      offset += part.length;
    }
    return strictJson(raw, cap, true);
  } finally {
    await reader.cancel();
  }
}
/** Remote alone pairs/reopens sessions. GET session is the Colab-owned echo
 * endpoint, not permission to inspect the Remote SDK's persistence schema. */
export async function register(mount: URL, sdk: RemoteSdk): Promise<Registration> {
  const remoteSession = await sdk.reopenSession();
  const session = await jsonResponse(
    await fetch(new URL('api/session', mount), { signal: AbortSignal.timeout(10_000) }),
    8192,
  );
  exactKeys(session, ['deviceId', 'publicKey', 'grantRevision', 'name']);
  requireValue(typeof session.deviceId === 'string');
  generatedId(session.deviceId);
  binary(session.publicKey, 32, 32);
  requireValue(typeof session.grantRevision === 'string' && typeof session.name === 'string');
  decimal(session.grantRevision);
  const deviceId = session.deviceId,
    keys = await deviceKeys(deviceId);
  const cert = async (purpose: 'sign' | 'enc', key: Uint8Array) => {
    const value = await sdk.certifyKey(purpose, key);
    requireValue(
      value.publicKey === encodeBinary(key) &&
        Number.isSafeInteger(value.issuedAtMs) &&
        value.issuedAtMs <= Date.now() &&
        Date.now() - value.issuedAtMs <= 10 * 60 * 1000,
    );
    binary(value.signature, 64, 64);
    return { publicKey: value.publicKey, issuedAtMs: value.issuedAtMs, signature: value.signature };
  };
  const response = await jsonResponse(
    await fetch(new URL('api/devices/register', mount), {
      signal: AbortSignal.timeout(10_000),
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        deviceId,
        sign: await cert('sign', keys.signPublic),
        enc: await cert('enc', keys.enc.publicKey()),
      }),
    }),
    32 * 1024,
  );
  exactKeys(response, ['chain', 'issuerStatement']);
  const chain = certificate.Chain.fromJson(text(JSON.stringify(response.chain))),
    issuer = statement.Envelope.fromJson(text(JSON.stringify(response.issuerStatement))),
    c = chain.certificate();
  requireValue(
    c.deviceId === deviceId &&
      c.issuerKind === 'member' &&
      equal(c.signingKey, keys.signPublic) &&
      equal(c.encryptionKey, keys.enc.publicKey()),
  );
  return { deviceId, keys, chain, issuer, remoteSession };
}
/** Call only with the root bound to the selected space, before trusting registration. */
export async function verifyRegistration(value: Registration, space: string, owner: Uint8Array) {
  requireValue((await deriveSpaceId(owner)) === space && value.issuer !== undefined);
  const verified = await value.issuer.verifyNext(space, owner, null),
    c = value.chain.certificate();
  requireValue(
    c.space === space &&
      c.issuerId === verified.head.ownerMember.id &&
      c.membershipRevision === '1' &&
      c.issuedAt <= Date.now() &&
      c.expiresAt > Date.now(),
  );
  await value.chain.verify(verified.head.hash, c, verified.head.ownerMember.signingKey);
  return verified.head;
}
