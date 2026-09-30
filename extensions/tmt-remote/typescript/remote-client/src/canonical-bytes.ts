/** Decoded-value byte builders only; these do not admit wire JSON or authorize effects. */
export interface Envelope {
  version: 1;
  profile: 'local-v1';
  kind: 'request' | 'response' | 'control';
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
  payload: Uint8Array;
}

export type Scope = 'agents.read' | 'results.own' | 'status.read' | 'talk.hold';
export interface Enrollment {
  profile: 'local-v1';
  machineId: string;
  windowId: string;
  offerId: string;
  serverChallenge: Uint8Array;
  clientNonce: Uint8Array;
  kind: 'addon' | 'cli';
  origin: string;
  name: string;
  publicKey: Uint8Array;
  agentIds: readonly string[];
  scopes: readonly Scope[];
  mode: 'hold';
}

const encoder = new TextEncoder();
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const ADDON_ORIGIN = /^chrome-extension:\/\/[a-p]{32}$/;
const SCOPES: readonly string[] = ['agents.read', 'results.own', 'status.read', 'talk.hold'];

function requireValue(condition: boolean, label: string): asserts condition {
  if (!condition) throw new Error(`Invalid ${label}.`);
}

function text(value: string): Uint8Array {
  requireValue(typeof value === 'string', 'text');
  // TextEncoder replaces lone surrogates; reject them before encoding instead.
  for (let index = 0; index < value.length; index++) {
    const unit = value.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(++index);
      requireValue(next >= 0xdc00 && next <= 0xdfff, 'UTF-8 surrogate');
    } else requireValue(unit < 0xdc00 || unit > 0xdfff, 'UTF-8 surrogate');
  }
  return encoder.encode(value);
}

function count(value: number): Uint8Array {
  requireValue(Number.isInteger(value) && value >= 0 && value <= 0xffffffff, 'u32 length');
  const result = new Uint8Array(4);
  new DataView(result.buffer).setUint32(0, value, false);
  return result;
}

function concat(parts: readonly Uint8Array[]): Uint8Array {
  const length = parts.reduce((sum, part) => sum + part.length, 0);
  requireValue(Number.isSafeInteger(length), 'combined length');
  const result = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    result.set(part, offset);
    offset += part.length;
  }
  return result;
}

function lp(value: Uint8Array): Uint8Array {
  return concat([count(value.length), value]);
}
const lpText = (value: string): Uint8Array => lp(text(value));

function uuid(value: string): void {
  requireValue(value.length === 36 && UUID.test(value), 'UUIDv4');
}

function binary(value: Uint8Array, length: number): Uint8Array {
  requireValue(value instanceof Uint8Array && value.length === length, `binary length ${length}`);
  return lp(value);
}

function origin(value: string): void {
  requireValue(
    text(value).length <= 128 &&
      (value === 'cli' || (value.length === 51 && ADDON_ORIGIN.test(value))),
    'origin',
  );
}

function sortedList(values: readonly string[], validate: (value: string) => void): Uint8Array {
  const parts = [count(values.length)];
  let previous: string | undefined;
  for (const value of values) {
    validate(value);
    // These lists contain ASCII UUIDs/scopes, so string order is UTF-8 byte order.
    requireValue(previous === undefined || previous < value, 'sorted distinct list');
    parts.push(lpText(value));
    previous = value;
  }
  return concat(parts);
}

/** Hash the exact supplied payload bytes, without parsing or reserializing them. */
export async function envelopeSigningBytes(value: Envelope): Promise<Uint8Array> {
  requireValue(value.version === 1 && value.profile === 'local-v1', 'envelope profile/version');
  requireValue(['request', 'response', 'control'].includes(value.kind), 'envelope kind');
  for (const id of [value.id, value.machineId, value.windowId, value.clientId]) uuid(id);
  if (value.kind === 'response') uuid(value.correlationId ?? '');
  else requireValue(value.correlationId === null, 'correlationId');
  requireValue(
    typeof value.sequence === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(value.sequence),
    'sequence spelling',
  );
  requireValue(
    BigInt(value.sequence).toString() === value.sequence &&
      BigInt(value.sequence) <= 18446744073709551615n,
    'sequence bound',
  );
  if (value.kind === 'control')
    requireValue(
      ['session.open', 'subscribe', 'ack'].includes(value.operation),
      'control operation',
    );
  if (value.kind === 'control' && value.operation === 'session.open') {
    requireValue(value.sessionId === 'new' && value.sequence === '0', 'session.open');
  } else {
    uuid(value.sessionId);
    requireValue(value.sequence !== '0', 'normal sequence');
  }
  requireValue(Number.isSafeInteger(value.timestampMs) && value.timestampMs >= 0, 'timestampMs');
  origin(value.origin);
  requireValue(text(value.operation).length > 0, 'operation');
  requireValue(value.payload instanceof Uint8Array, 'payload bytes');
  // Snapshot all caller-owned data before the asynchronous hash boundary (including Buffers).
  const payload = new Uint8Array(value.payload);
  const fields = [
    'tmt-message-v1',
    String(value.version),
    value.profile,
    value.kind,
    value.id,
    value.correlationId ?? '',
    value.machineId,
    value.windowId,
    value.clientId,
    value.sessionId,
    value.sequence,
    String(value.timestampMs),
    value.origin,
    value.operation,
  ].map(lpText);
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', payload));
  return concat([...fields, lp(digest)]);
}

/** Encode a decoded enrollment candidate; no key validity, proof or authority is established. */
export function enrollmentSigningBytes(value: Enrollment): Uint8Array {
  requireValue(value.profile === 'local-v1' && value.mode === 'hold', 'enrollment profile/mode');
  for (const id of [value.machineId, value.windowId, value.offerId]) uuid(id);
  requireValue(value.kind === 'addon' || value.kind === 'cli', 'enrollment kind');
  origin(value.origin);
  requireValue((value.kind === 'cli') === (value.origin === 'cli'), 'kind/origin');
  const name = text(value.name);
  requireValue(
    name.length >= 1 &&
      name.length <= 64 &&
      value.name.trim().length > 0 &&
      !Array.from(value.name).some((character) => {
        const point = character.codePointAt(0)!;
        return point <= 31 || (point >= 127 && point <= 159);
      }),
    'name',
  );
  requireValue(value.agentIds.length <= 256, 'agent count');
  return concat([
    lpText('tmt-local-pair-v1'),
    lpText(value.profile),
    lpText(value.machineId),
    lpText(value.windowId),
    lpText(value.offerId),
    binary(value.serverChallenge, 16),
    binary(value.clientNonce, 16),
    lpText(value.kind),
    lpText(value.origin),
    lp(name),
    binary(value.publicKey, 32),
    sortedList(value.agentIds, uuid),
    sortedList(value.scopes, (scope) => requireValue(SCOPES.includes(scope), 'scope')),
    lpText(value.mode),
  ]);
}

/** Frame already canonical enrollment bytes and an already computed full MAC; do not compute it. */
export function enrollmentPossessionSigningBytes(
  enrollment: Uint8Array,
  mac: Uint8Array,
): Uint8Array {
  requireValue(enrollment instanceof Uint8Array, 'enrollment bytes');
  return concat([lpText('tmt-local-pair-possession-v1'), lp(enrollment), binary(mac, 32)]);
}
