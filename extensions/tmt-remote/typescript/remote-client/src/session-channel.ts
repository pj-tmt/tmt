import { base64urlBytes, envelopeSigningBytes } from './canonical-bytes.js';
import type { DeviceKey, Paired, Session } from './device.js';

const owned = (bytes: Uint8Array): Uint8Array<ArrayBuffer> => new Uint8Array(bytes);
export async function verifyEd25519(
  publicKey: Uint8Array,
  message: Uint8Array,
  signature: Uint8Array,
): Promise<boolean> {
  const key = await crypto.subtle.importKey('raw', owned(publicKey), 'Ed25519', false, ['verify']);
  return (
    signature.length === 64 &&
    crypto.subtle.verify('Ed25519', key, owned(signature), owned(message))
  );
}

/** Private transport ownership; copying a public Session does not copy its signer. */
export interface Channel {
  paired: Paired;
  key: DeviceKey;
  windowId: string;
  sessionId: string;
  fetch: typeof fetch;
  clientSequence: bigint;
  machineSequence: bigint;
  tail: Promise<void>;
  ended: boolean;
  uncertainSequence?: bigint | 'unavailable';
}
const channels = new WeakMap<Session, Channel>();
export function registerChannel(
  session: Session,
  paired: Paired,
  key: DeviceKey,
  windowId: string,
  fetch: typeof globalThis.fetch,
): void {
  channels.set(session, {
    paired: { ...paired, machinePublicKey: new Uint8Array(paired.machinePublicKey) },
    key,
    windowId,
    sessionId: session.sessionId,
    fetch,
    clientSequence: 1n,
    machineSequence: 1n,
    tail: Promise.resolve(),
    ended: false,
  });
}
export const channelFor = (session: Session): Channel | undefined => channels.get(session);

/** Authenticate exact bytes and correlation before the operation parses its payload. */
export async function verifyResponse(
  value: unknown,
  paired: Paired,
  expected: { id: string; windowId: string; operation: string; sessionId?: string; after: bigint },
): Promise<{ payload: Uint8Array; sequence: bigint; sessionId: string }> {
  function valid(condition: boolean): asserts condition {
    if (!condition) throw new Error('Invalid machine response.');
  }
  valid(typeof value === 'object' && value !== null && !Array.isArray(value));
  const reply = value as Record<string, unknown>;
  const text = (name: string): string => {
    const value = reply[name];
    valid(typeof value === 'string');
    return value;
  };
  valid(
    reply.version === 1 &&
      text('profile') === 'local-v1' &&
      text('kind') === 'response' &&
      text('correlationId') === expected.id &&
      text('machineId') === paired.machineId &&
      text('windowId') === expected.windowId &&
      text('clientId') === paired.clientId &&
      text('origin') === paired.origin &&
      text('operation') === expected.operation &&
      Number.isSafeInteger(reply.timestampMs),
  );
  const sessionId = text('sessionId');
  valid(expected.sessionId === undefined || sessionId === expected.sessionId);
  const payloadText = text('payload');
  const payload = base64urlBytes(payloadText, Math.floor((payloadText.length * 6) / 8));
  const signed = await envelopeSigningBytes({
    version: 1,
    profile: 'local-v1',
    kind: 'response',
    id: text('id'),
    correlationId: expected.id,
    machineId: paired.machineId,
    windowId: expected.windowId,
    clientId: paired.clientId,
    sessionId,
    sequence: text('sequence'),
    timestampMs: reply.timestampMs as number,
    origin: paired.origin,
    operation: expected.operation,
    payload,
  });
  const sequence = BigInt(text('sequence'));
  valid(sequence > expected.after);
  valid(
    await verifyEd25519(paired.machinePublicKey, signed, base64urlBytes(text('signature'), 64)),
  );
  return { payload, sequence, sessionId };
}
