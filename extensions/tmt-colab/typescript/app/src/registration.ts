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
  /** With `previous` and `retry: 'bounded'` the SDK admits again for that Session and throws its
   * typed `ReopenSessionError`; without options it is the one-shot boot reopen. */
  reopenSession(
    previous?: unknown,
    options?: { retry?: 'bounded'; signal?: AbortSignal },
  ): Promise<unknown>;
  certifyKey(purpose: 'sign' | 'enc', publicKey: Uint8Array): Promise<CertifiedKey>;
  transportUrl(session: unknown, url: string | URL): string;
}
/** Why the SDK's bounded re-admission failed (Remote R2, `ReopenSessionError.reason`). */
export const REOPEN_REASONS = [
  'unreachable',
  'unconfirmed',
  'transient',
  'unpaired',
  'mismatch',
  'revoked',
  'expired',
  'capacity',
] as const;
export type ReopenReason = (typeof REOPEN_REASONS)[number];
/** The SDK's typed refusal of a fresh open, normalized here because its class arrives at run time. */
export class ReopenFailedError extends Error {
  constructor(
    readonly reason: ReopenReason,
    cause?: unknown,
  ) {
    super(`REMOTE_REOPEN_${reason.toUpperCase()}`, { cause });
  }
}
/** A fresh open refused because the paired device is at its Remote session limit (#2585). Remote
 * never evicts an attached session, so nothing changes until a session frees. */
export class SessionLimitError extends Error {
  constructor(
    readonly limit: number,
    readonly settingsUrl?: string,
  ) {
    super('REMOTE_SESSION_LIMIT');
  }
}
/** The settings link is shown as an anchor, so only a web URL is kept. */
function webUrl(value: unknown): string | undefined {
  if (typeof value !== 'string' || !URL.canParse(value)) return undefined;
  const { protocol } = new URL(value);
  return protocol === 'https:' || protocol === 'http:' ? value : undefined;
}
/**
 * The verified limit refusal behind a failed open, or undefined. The boot open throws the SDK's
 * `RefusalError` itself; a bounded re-admission wraps it as the `cause` of its `capacity`
 * `ReopenSessionError`. Both classes arrive at run time, so they are recognised by name, and a
 * limit that is not a positive integer is not trusted.
 */
export function sessionLimit(error: unknown): SessionLimitError | undefined {
  for (let depth = 0, next = error; depth < 3 && next instanceof Error; depth++) {
    if (next.name === 'RefusalError') {
      const { code, limit, settingsUrl } = next as Error & Record<string, unknown>;
      return code === 'REMOTE_SESSION_LIMIT' && Number.isSafeInteger(limit) && (limit as number) > 0
        ? new SessionLimitError(limit as number, webUrl(settingsUrl))
        : undefined;
    }
    next = next.cause;
  }
  return undefined;
}
function reopenFailure(error: unknown): unknown {
  if (!(error instanceof Error) || error.name !== 'ReopenSessionError') return error;
  const { reason } = error as Error & { reason?: unknown };
  return REOPEN_REASONS.find((known) => known === reason)
    ? new ReopenFailedError(reason as ReopenReason, error)
    : error;
}
/** Re-admission of a Session that ended while the device is still paired. */
export interface Readmission {
  previous: unknown;
  signal?: AbortSignal;
}
export interface Registration {
  remoteSession?: unknown;
  /** Tab-bound mounted WebSocket URL from Remote's SDK; never navigated to or logged. */
  syncUrl?: string;
  deviceName?: string;
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
/** An HTTP admission refusal retains its bounded body; a status alone is not revoke proof. */
export class AdmissionRefusedError extends Error {
  constructor(
    readonly status: number,
    readonly body: string,
    readonly code?: string,
  ) {
    super(code ?? `Colab registration unavailable (${status})`);
  }
}
export async function jsonResponse(response: Response, cap: number) {
  // Stream limits apply even when the server omits or lies about Content-Length.
  const reader = response.body?.getReader();
  if (!reader) {
    if (!response.ok) throw new AdmissionRefusedError(response.status, '');
    throw new Error('Empty Colab response');
  }
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
    if (!response.ok) {
      let code: string | undefined;
      try {
        const refusal = strictJson(raw, cap, true);
        if (
          typeof refusal === 'object' &&
          refusal !== null &&
          !Array.isArray(refusal) &&
          'code' in refusal &&
          typeof refusal.code === 'string'
        )
          code = refusal.code;
      } catch {
        // An empty or unrecognized refusal is still an admission failure, not revoke proof.
      }
      throw new AdmissionRefusedError(response.status, new TextDecoder().decode(raw), code);
    }
    return strictJson(raw, cap, true);
  } finally {
    await reader.cancel();
  }
}
/** Remote alone pairs/reopens sessions. GET session is the Colab-owned echo
 * endpoint, not permission to inspect the Remote SDK's persistence schema. */
export async function register(
  mount: URL,
  sdk: RemoteSdk,
  readmit?: Readmission,
): Promise<Registration> {
  const remoteSession = await (
    readmit
      ? sdk.reopenSession(readmit.previous, { retry: 'bounded', signal: readmit.signal })
      : sdk.reopenSession()
  ).catch((error: unknown) => {
    throw sessionLimit(error) ?? (readmit ? reopenFailure(error) : error);
  });
  const sync = new URL('sync', mount);
  sync.protocol = sync.protocol === 'https:' ? 'wss:' : 'ws:';
  const syncUrl = sdk.transportUrl(remoteSession, sync);
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
  return { deviceId, keys, chain, issuer, remoteSession, syncUrl, deviceName: session.name };
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
