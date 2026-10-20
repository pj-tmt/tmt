/** Closed JSON value checks shared by the structured page values (`shell`, `work`). No target
 * access, storage or authority: a value is valid or the whole action is refused. Mirrors
 * `closed.rs`. */
const utf8 = new TextEncoder();
const generatedId = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const coreId = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export function requireValue(ok: unknown): asserts ok {
  if (!ok) throw new Error('Invalid structured value');
}
export function object(value: unknown): Record<string, unknown> {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  return value as Record<string, unknown>;
}
/** Exactly the required keys and some of the optional ones; an optional key is never `null`. */
export function keys(value: unknown, required: string[], optional: string[] = []) {
  const map = object(value);
  requireValue(required.every((key) => Object.hasOwn(map, key)));
  requireValue(
    Object.entries(map).every(
      ([key, value]) => (required.includes(key) || optional.includes(key)) && value !== null,
    ),
  );
  return map;
}
/** A canonical generated UUID. */
export function id(value: unknown): asserts value is string {
  requireValue(typeof value === 'string' && generatedId.test(value));
}
/** A Core identity UUID: not necessarily version 4, and never the nil UUID. */
export function coreIdentity(value: unknown): asserts value is string {
  requireValue(
    typeof value === 'string' &&
      coreId.test(value) &&
      value !== '00000000-0000-0000-0000-000000000000',
  );
}
export function byteLength(value: string): number {
  return utf8.encode(value).length;
}
/** Inert text of at most `bytes` UTF-8 bytes: no control characters, no unpaired surrogates. */
export function plain(value: unknown, bytes: number): asserts value is string {
  requireValue(
    typeof value === 'string' && byteLength(value) <= bytes && !/[\p{Cc}\p{Cs}]/u.test(value),
  );
}
/** Like `plain`, but a line feed is allowed: a spec reads as paragraphs. */
export function multiline(value: unknown, bytes: number): asserts value is string {
  requireValue(
    typeof value === 'string' &&
      byteLength(value) <= bytes &&
      !/[\p{Cc}\p{Cs}]/u.test(value.replaceAll('\n', '')),
  );
}
/** Non-empty inert text of at most `bytes` UTF-8 bytes. */
export function named(value: unknown, bytes: number): asserts value is string {
  plain(value, bytes);
  requireValue(value.length > 0);
}
/** Compact sorted ASCII field/UUID keys; only closed ASCII names occur, so lexical sorting is shared
 * with the native side. */
export function canonical(value: unknown): string {
  const sorted = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(sorted);
    if (value !== null && typeof value === 'object')
      return Object.fromEntries(
        Object.entries(value)
          .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
          .map(([key, value]) => [key, sorted(value)]),
      );
    return value;
  };
  return JSON.stringify(sorted(value));
}
