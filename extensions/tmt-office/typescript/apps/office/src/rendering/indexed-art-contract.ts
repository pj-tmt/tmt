/** Pure predicates shared by the two strict indexed-art projections. */
export const INDEXED_ART_LIMITS = {
  keyBytes: 32,
  labelBytes: 80,
  creditBytes: 120,
  licenseBytes: 64,
  paletteEntries: 16,
} as const;
export function indexedLicense(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length >= 1 &&
    value.length <= INDEXED_ART_LIMITS.licenseBytes &&
    Array.from(value).every(
      (character) =>
        (character >= 'A' && character <= 'Z') ||
        (character >= 'a' && character <= 'z') ||
        (character >= '0' && character <= '9') ||
        character === '.' ||
        character === '+' ||
        character === '-'
    )
  );
}

export function indexedArtKey(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    new TextEncoder().encode(value).length <= INDEXED_ART_LIMITS.keyBytes &&
    /^[a-z][a-z0-9-]{0,31}$/.test(value)
  );
}

export function indexedPalette(
  value: unknown,
  maxEntries: number = INDEXED_ART_LIMITS.paletteEntries
): value is string[] {
  return (
    Array.isArray(value) &&
    value.length >= 1 &&
    value.length <= maxEntries &&
    value[0] === '#00000000' &&
    value.slice(1).every((color) => typeof color === 'string' && /^#[0-9a-f]{6}ff$/.test(color))
  );
}

export function indexedRaster(
  value: unknown,
  paletteLength: number,
  limits: { width?: number; height?: number; maxSide: number; maxCells: number; indexWidth?: 1 | 2 }
): string[] | undefined {
  if (!Array.isArray(value) || value.length < 1 || value.length > limits.maxSide) return;
  if (limits.height !== undefined && value.length !== limits.height) return;
  const indexWidth = limits.indexWidth ?? 1;
  const width = typeof value[0] === 'string' ? value[0].length / indexWidth : 0;
  if (
    width < 1 ||
    !Number.isInteger(width) ||
    width > limits.maxSide ||
    (limits.width !== undefined && width !== limits.width) ||
    width * value.length > limits.maxCells
  )
    return;
  return value.every(
    (row) =>
      typeof row === 'string' &&
      row.length === width * indexWidth &&
      /^[0-9a-f]+$/.test(row) &&
      Array.from({ length: width }, (_, x) =>
        Number.parseInt(row.slice(x * indexWidth, (x + 1) * indexWidth), 16)
      ).every((index) => index < paletteLength)
  )
    ? (value as string[])
    : undefined;
}
