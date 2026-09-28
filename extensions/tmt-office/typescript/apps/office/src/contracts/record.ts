/** Structural admission shared by data-only Office documents. */
export function canonicalUuid(value: unknown): string {
  if (
    typeof value !== 'string' ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value) ||
    value === '00000000-0000-0000-0000-000000000000'
  )
    throw new Error('Invalid identifier.');
  return value;
}

export function uuidIdentifier(value: unknown): string {
  if (
    typeof value !== 'string' ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value)
  )
    throw new Error('Invalid identifier.');
  return value;
}

export function exactRecord(
  value: unknown,
  keys: string[],
  label: string
): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error(`Invalid ${label}.`);
  const object = value as Record<string, unknown>;
  if (
    Object.keys(object).length !== keys.length ||
    !keys.every((key) => Object.hasOwn(object, key))
  )
    throw new Error(`Invalid ${label} fields.`);
  return object;
}

export function boundedText(value: unknown, maxBytes: number): value is string {
  if (typeof value !== 'string') return false;
  const bytes = new TextEncoder().encode(value).length;
  return (
    bytes >= 1 &&
    bytes <= maxBytes &&
    !Array.from(value).some((character) => {
      const code = character.codePointAt(0)!;
      return code <= 0x1f || (code >= 0x7f && code <= 0x9f) || (code >= 0xd800 && code <= 0xdfff);
    })
  );
}
