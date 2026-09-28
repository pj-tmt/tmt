import { expect, it } from 'vitest';
import vectors from '../../../../../contracts/prop-customization-vectors.json';
import { decodePropPack } from './prop-contract.js';
import { encodeLayout, localLayoutValue, validLocalLayout } from '../blocks/block-contract.js';

it.each(vectors.cases)('customization admission: $name', ({ value, valid }) => {
  const candidate = structuredClone(vectors.pack);
  const input = { ...candidate, props: [{ ...candidate.props[0], customization: value }] };
  if (valid) expect(decodePropPack(input).props[0]!.customization).toEqual(value);
  else expect(() => decodePropPack(input)).toThrow();
});

it.each(vectors.placementCases)('placement admission: $name', ({ value, valid }) => {
  const object = {
    prop: `sha256:${'1'.repeat(64)}/rug`,
    footprint: { width: 2, height: 1 },
    x: 0,
    y: 0,
    rotation: 0,
    customization: value,
  };
  const layout = { version: 3, objects: [object] };
  expect(validLocalLayout(layout)).toBe(valid);
  expect(validLocalLayout({ ...layout, version: 2 })).toBe(false);
  if (validLocalLayout(layout)) {
    expect(localLayoutValue(layout.objects)).toEqual(layout);
    expect(() => encodeLayout(layout.objects)).toThrow();
  }
});

it('rejects a tint channel absent from one direction and capabilities on legacy art', () => {
  const prop = {
    ...structuredClone(vectors.pack.props[0]!),
    customization: { tint: { indices: [1] } },
  };
  prop.frames[3] = ['0202', '0202', '0202', '0000'];
  expect(() => decodePropPack({ ...vectors.pack, props: [prop] })).toThrow();
  const { frames: _, ...legacy } = prop;
  expect(() =>
    decodePropPack({ ...vectors.pack, formatVersion: 1, props: [{ ...legacy, pixels: ['1'] }] })
  ).toThrow();
});
