export function valid(condition: unknown): asserts condition {
  if (!condition) throw new Error('Invalid management response.');
}
export function input(condition: unknown): asserts condition {
  if (!condition) throw new TypeError('Invalid management input.');
}
export function object(value: unknown): Record<string, unknown> {
  valid(typeof value === 'object' && value !== null && !Array.isArray(value));
  return value as Record<string, unknown>;
}
export function exact(value: Record<string, unknown>, keys: string[]): void {
  valid(
    Object.keys(value).length === keys.length && keys.every((key) => Object.hasOwn(value, key)),
  );
}
export const integer = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;

/** The signed member service accepts nonblank names of at most 64 UTF-8 bytes. */
export function memberName(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.trim().length > 0 &&
    new TextEncoder().encode(value).length <= 64 &&
    ![...value].some((character) => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127)
  );
}
