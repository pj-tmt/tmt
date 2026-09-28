/** Synthetic admission boundaries, not production art or a second pack decoder. */
export const CAPACITY_PACK_BYTES = 512 * 1024;
export const CAPACITY_PACK_CELLS = 131_072;

export function capacityPropSource(index: number, propCount: 8 | 16 = 16) {
  const palette = [
    '#00000000',
    ...Array.from(
      { length: 255 },
      (_, color) => `#${((color + 1) * 0x010101).toString(16).padStart(6, '0')}ff`
    ),
  ];
  const frameDepth = 256 / propCount;
  const props = Array.from({ length: propCount }, (_, prop) => ({
    key: `tile-${prop}`,
    label: `Capacity tile ${prop}`,
    footprint: { width: 16, height: frameDepth / 8 },
    frames: Array.from({ length: 4 }, (_, direction) => {
      const width = direction % 2 ? frameDepth : 128;
      const height = direction % 2 ? 128 : frameDepth;
      return Array.from({ length: height }, (_, y) =>
        Array.from({ length: width }, (_, x) =>
          (1 + ((x * 17 + y * 31 + direction * 53 + prop * 7) % 255)).toString(16).padStart(2, '0')
        ).join('')
      );
    }),
  }));
  // Both catalog scenarios contain 131,072 real cells per pack. Legal whitespace
  // separately reaches the exact file-byte ceiling without inventing fields.
  const source = JSON.stringify({
    formatVersion: 2,
    label: `Capacity pack ${index}`,
    credit: 'Synthetic capacity fixture',
    license: 'CC0-1.0',
    palette,
    props,
  });
  if (Buffer.byteLength(source) > CAPACITY_PACK_BYTES)
    throw new Error('Capacity fixture overflow.');
  return source.padEnd(CAPACITY_PACK_BYTES, ' ');
}
