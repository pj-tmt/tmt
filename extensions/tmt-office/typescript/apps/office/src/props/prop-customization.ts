import { boundedText, exactRecord } from '../contracts/record.js';

export interface PropCustomization {
  tint?: string;
  text?: string;
}

export function validPropCustomization(value: unknown): value is PropCustomization {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  const keys = Object.keys(record);
  return (
    keys.length > 0 &&
    keys.every((key) => key === 'tint' || key === 'text') &&
    (!Object.hasOwn(record, 'tint') ||
      (typeof record.tint === 'string' && /^#[0-9a-f]{6}$/.test(record.tint))) &&
    (!Object.hasOwn(record, 'text') ||
      (boundedText(record.text, 64) &&
        record.text.trim().length > 0 &&
        Array.from(record.text).length <= 24 &&
        !/[\u2028-\u202e\u2066-\u2069]/u.test(record.text)))
  );
}

export function permitsCustomization(
  capabilities: PropCapabilities | undefined,
  value?: PropCustomization
) {
  return (
    !value ||
    Boolean(
      capabilities && (!value.tint || capabilities.tint) && (!value.text || capabilities.text)
    )
  );
}

export interface TextRegion {
  x: number;
  y: number;
  width: number;
  height: number;
}
export interface PropCapabilities {
  tint?: { indices: number[] };
  text?: { regions: [TextRegion, TextRegion, TextRegion, TextRegion]; color: string };
}

/** Rasters have already passed indexed-art admission before capabilities are decoded. */
export function decodePropCapabilities(
  value: unknown,
  frames: readonly string[][],
  paletteLength: number
): PropCapabilities {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Invalid prop customization.');
  const keys = Object.keys(value);
  if (!keys.length || keys.some((key) => key !== 'tint' && key !== 'text'))
    throw new Error('Invalid prop customization fields.');
  const input = value as Record<string, unknown>;
  const result: PropCapabilities = {};
  if (Object.hasOwn(input, 'tint')) {
    const tint = exactRecord(input.tint, ['indices'], 'tint capability');
    if (
      !Array.isArray(tint.indices) ||
      !tint.indices.length ||
      tint.indices.some(
        (index) => !Number.isInteger(index) || index < 1 || index >= paletteLength
      ) ||
      new Set(tint.indices).size !== tint.indices.length
    )
      throw new Error('Invalid tint indices.');
    const indices = new Set<number>(tint.indices);
    const used = new Set<number>();
    for (const frame of frames) {
      let frameHasTint = false;
      for (const row of frame) {
        for (let offset = 0; offset < row.length; offset += 2) {
          const index = Number.parseInt(row.slice(offset, offset + 2), 16);
          if (indices.has(index)) {
            used.add(index);
            frameHasTint = true;
          }
        }
      }
      if (!frameHasTint) throw new Error('Every frame must use its tint channel.');
    }
    if (used.size !== indices.size) throw new Error('Tint indices must appear in the artwork.');
    result.tint = { indices: [...indices] };
  }
  if (Object.hasOwn(input, 'text')) {
    const text = exactRecord(input.text, ['regions', 'color'], 'text capability');
    if (
      typeof text.color !== 'string' ||
      !/^#[0-9a-f]{6}$/.test(text.color) ||
      !Array.isArray(text.regions) ||
      text.regions.length !== 4
    )
      throw new Error('Invalid text capability.');
    const regions = text.regions.map((input, index) => {
      const region = exactRecord(input, ['x', 'y', 'width', 'height'], 'text region');
      if (
        !Object.values(region).every((value) => Number.isInteger(value) && Number(value) >= 0) ||
        Number(region.width) < 1 ||
        Number(region.height) < 1 ||
        Number(region.x) + Number(region.width) > frames[index]![0]!.length / 2 ||
        Number(region.y) + Number(region.height) > frames[index]!.length
      )
        throw new Error('Text regions must fit their directional frame.');
      return region as unknown as TextRegion;
    });
    result.text = {
      color: text.color,
      regions: regions as [TextRegion, TextRegion, TextRegion, TextRegion],
    };
  }
  return result;
}
