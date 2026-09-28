import { describe, expect, it } from 'vitest';
import vectors from '../../../../../../../contracts/office/block-v1.vectors.json' with { type: 'json' };
import propVectors from '../../../../../../../contracts/office/prop-block-vectors.json' with { type: 'json' };
import {
  footprint,
  sameLayout,
  validFurniture,
  validLayout,
  encodeLayout,
  decodeLayout,
  builtinFurniture,
} from './block-contract.js';

describe('bounded decoration contract', () => {
  it.each(vectors)('$name', ({ valid, item, stored }) => {
    if (valid) {
      expect(item).not.toBeNull();
      const decoded = decodeLayout([stored]);
      expect(validFurniture(decoded[0])).toBe(true);
      expect(encodeLayout(decoded)).toEqual([stored]);
      expect(decoded[0]).toMatchObject({
        x: item!.x,
        y: item!.y,
        rotation: item!.rotation,
      });
    } else expect(() => decodeLayout([stored])).toThrow();
  });
  it('checks the last slot, the budget and empty layouts', () => {
    const item = builtinFurniture('desk', 0, 0, 0);
    expect(validLayout([])).toBe(true);
    expect(validLayout(Array(16).fill(item))).toBe(true);
    expect(validLayout([...Array(15).fill(item), { ...item, x: 32 }])).toBe(false);
    expect(validLayout(Array(17).fill(item))).toBe(false);
    expect(validLayout(Array(16))).toBe(false);
  });
  it('uses the rotated footprint for rendering', () => {
    expect(footprint(builtinFurniture('rug', 0, 0, 1))).toEqual({ width: 4, height: 6 });
  });
  it('equality preserves paint order but ignores JSON key order', () => {
    const first = builtinFurniture('desk', 0, 0, 0);
    const second = builtinFurniture('rug', 0, 0, 0);
    expect(sameLayout([first], [{ ...first, rotation: 0, y: 0, x: 0 }])).toBe(true);
    expect(sameLayout([first, second], [second, first])).toBe(false);
  });
  it.each(propVectors.layoutCases)('$name', ({ valid, value }) => {
    const exactEnvelope =
      Object.keys(value).length === 2 &&
      Object.hasOwn(value, 'version') &&
      Object.hasOwn(value, 'objects') &&
      value.version === 2;
    expect(exactEnvelope && validLayout(value.objects)).toBe(valid);
  });
  it('accepts the shared full local object capacity', () => {
    const { capacity } = propVectors;
    expect(validLayout(Array(capacity.count).fill(capacity.placement))).toBe(true);
    expect(validLayout(Array(capacity.count + 1).fill(capacity.placement))).toBe(false);
  });
});
