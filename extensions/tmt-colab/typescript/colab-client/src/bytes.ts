/** Colab value admission and the Remote-owned LP primitive, without authority lookup. */
export type Bytes = Uint8Array<ArrayBuffer>;
export function requireValue(ok: boolean): asserts ok {
  if (!ok) throw new Error('Invalid colab-v1 value or proof');
}
export function copy(value: Uint8Array, size?: number): Bytes {
  requireValue(value instanceof Uint8Array && (size === undefined || value.length === size));
  return new Uint8Array(value);
}
export function text(value: string): Bytes {
  requireValue(typeof value === 'string');
  for (let i = 0; i < value.length; i++) {
    const n = value.charCodeAt(i);
    if (n >= 0xd800 && n <= 0xdbff) {
      const next = value.charCodeAt(++i);
      requireValue(next >= 0xdc00 && next <= 0xdfff);
    } else requireValue(n < 0xdc00 || n > 0xdfff);
  }
  return new TextEncoder().encode(value);
}
export function decodeText(value: Uint8Array): string {
  return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(value);
}
export function equal(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && a.every((n, i) => n === b[i]);
}
export function concat(...values: Uint8Array[]): Bytes {
  const length = values.reduce((n, v) => n + v.length, 0);
  requireValue(Number.isSafeInteger(length) && length <= 0xffffffff);
  const out = new Uint8Array(length);
  let offset = 0;
  for (const v of values) {
    out.set(v, offset);
    offset += v.length;
  }
  return out;
}
export function frame(...values: Uint8Array[]): Bytes {
  return concat(
    ...values.map((v) => {
      const n = new Uint8Array(4);
      new DataView(n.buffer).setUint32(0, v.length);
      return concat(n, v);
    }),
  );
}
export function fields(raw: Uint8Array, count: number, max = 1024): Bytes[] {
  requireValue(raw.length <= max);
  const bytes = copy(raw),
    out: Bytes[] = [];
  let offset = 0;
  for (let i = 0; i < count; i++) {
    requireValue(offset + 4 <= bytes.length);
    const n = new DataView(bytes.buffer).getUint32(offset);
    offset += 4;
    requireValue(offset + n <= bytes.length);
    out.push(bytes.slice(offset, offset + n));
    offset += n;
  }
  requireValue(offset === bytes.length);
  return out;
}
export function generatedId(value: string): void {
  requireValue(
    typeof value === 'string' &&
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value),
  );
}
export function spaceId(value: string): void {
  requireValue(typeof value === 'string' && /^[a-z2-7]{32}$/.test(value));
}
export function decimal(value: string, zero = false): bigint {
  requireValue(typeof value === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(value));
  const n = BigInt(value);
  requireValue(n <= 0xffffffffffffffffn && (zero || n > 0n));
  return n;
}
export function time(value: number): void {
  requireValue(Number.isSafeInteger(value) && value >= 0 && !Object.is(value, -0));
}
export function namespace(value: string): void {
  requireValue(value === 'content' || value === 'own');
}
export function encodeBinary(value: Uint8Array): string {
  let raw = '';
  for (let i = 0; i < value.length; i += 8192)
    raw += String.fromCharCode(...value.subarray(i, i + 8192));
  return btoa(raw).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}
export function binary(value: unknown, max: number, exact?: number): Bytes {
  requireValue(
    typeof value === 'string' &&
      value.length <= Math.ceil(max / 3) * 4 &&
      /^[A-Za-z0-9_-]*$/.test(value),
  );
  const raw = atob(value.replaceAll('-', '+').replaceAll('_', '/'));
  requireValue(raw.length <= max && (exact === undefined || raw.length === exact));
  const out = Uint8Array.from(raw, (c) => c.charCodeAt(0));
  requireValue(encodeBinary(out) === value);
  return out;
}
export function exactKeys(
  value: unknown,
  names: readonly string[],
): asserts value is Record<string, unknown> {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  const keys = Reflect.ownKeys(value);
  requireValue(
    keys.length === names.length && keys.every((k) => typeof k === 'string' && names.includes(k)),
  );
}
export function coreId(value: string): void {
  requireValue(
    typeof value === 'string' &&
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value) &&
      value !== '00000000-0000-0000-0000-000000000000',
  );
}
export function idList(ids: readonly string[], core = false): Bytes {
  requireValue(Array.isArray(ids) && ids.length <= 256);
  const count = new Uint8Array(4);
  new DataView(count.buffer).setUint32(0, ids.length);
  const entries: Bytes[] = [];
  let previous: string | undefined;
  for (const id of ids) {
    if (core) coreId(id);
    else generatedId(id);
    requireValue(previous === undefined || previous < id);
    previous = id;
    entries.push(frame(text(id)));
  }
  const out = concat(count, ...entries);
  requireValue(out.length <= 10244);
  return out;
}
