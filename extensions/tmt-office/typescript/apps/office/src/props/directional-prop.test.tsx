import { render } from '@testing-library/react';
import { expect, it } from 'vitest';
import sample from '../../../../../../../contracts/office/prop-pack-v2-sample.tmtprop.json' with { type: 'json' };
import customizationVectors from '../../../../../../../contracts/office/prop-customization-vectors.json';
import { indexedCells } from '../rendering/indexed-art.js';
import { indexedPalette } from '../rendering/indexed-art-contract.js';
import { BUILTIN_PACK, decodePropPack, propFrame } from './prop-contract.js';
import { IndexedProp } from './indexed-prop.js';

it('selects four distinct upright frames with rotated footprint dimensions', () => {
  const pack = decodePropPack(sample);
  const prop = pack.props[0]!;
  for (const rotation of [0, 1, 2, 3]) {
    const frame = propFrame(pack, prop, rotation);
    expect(frame.pixels).toEqual(sample.props[0]!.frames[rotation]);
    expect(frame.rotation).toBe(0);
    expect(frame.frame).toBe(rotation);
    expect(frame.indexWidth).toBe(2);
    expect([frame.width, frame.height]).toEqual(rotation % 2 ? [1, 2] : [2, 1]);
    const view = render(
      <svg>
        <IndexedProp pack={pack} prop={prop} rotation={rotation} />
      </svg>
    );
    expect(view.container.querySelector('g')?.getAttribute('transform')).toContain('rotate(0)');
    expect(view.container.querySelector('svg svg')?.getAttribute('viewBox')).toBe(
      rotation % 2 ? '0 0 2 4' : '0 0 4 2'
    );
    view.unmount();
  }
});

it('interprets byte indices as pixels, including indices above the v1 range', () => {
  const pack = decodePropPack(sample);
  const cells = Array.from(indexedCells(propFrame(pack, pack.props[0]!)));
  expect(cells).toEqual([
    { x: 1, y: 0, color: '#101010ff' },
    { x: 2, y: 0, color: '#010101ff' },
    { x: 3, y: 0, color: '#010101ff' },
    { x: 0, y: 1, color: '#010101ff' },
    { x: 1, y: 1, color: '#010101ff' },
  ]);
  expect(indexedPalette(sample.palette)).toBe(false);
});

it('retains the single-frame rotation and digit width of existing props', () => {
  const prop = BUILTIN_PACK.props[0]!;
  const frame = propFrame(BUILTIN_PACK, prop, 1);
  expect(frame.rotation).toBe(1);
  expect(frame.frame).toBe(0);
  expect(frame.pixels).toBe(prop.pixels);
  expect(frame.indexWidth).toBe(1);
  expect([frame.width, frame.height]).toEqual([4, 2]);
});

it('accepts the complete v2 cell budget and rejects an otherwise-valid extra row', () => {
  const value = {
    ...structuredClone(sample),
    props: Array.from({ length: 16 }, (_, index) => ({
      ...structuredClone(sample.props[0]!),
      key: `capacity-${index}`,
      frames: Array.from({ length: 4 }, () => Array<string>(16).fill('01'.repeat(128))),
    })),
  };
  expect(decodePropPack(value).props).toHaveLength(16);
  // 16 props * 4 frames * 16 rows * 128 cells = 131,072. The extra row
  // remains within the per-frame dimensions, isolating the pack-total gate.
  value.props[0]!.frames[0]!.push('01'.repeat(128));
  expect(() => decodePropPack(value)).toThrow();
});

it('tints only declared indices without mutating shared art and projects inert text for every direction', () => {
  const source = structuredClone(customizationVectors.pack);
  const capabilities = customizationVectors.cases[1]!.value;
  if (!capabilities || !('text' in capabilities))
    throw new Error('Missing text capability fixture');
  const input = {
    ...source,
    props: [
      {
        ...source.props[0],
        customization: {
          tint: { indices: [1] },
          text: capabilities.text,
        },
      },
    ],
  };
  const pack = decodePropPack(input);
  const prop = pack.props[0]!;
  for (let rotation = 0; rotation < 4; rotation++) {
    const red = propFrame(pack, prop, rotation, { tint: '#ff0000', text: '<b>Office</b>' });
    const blue = propFrame(pack, prop, rotation, { tint: '#0000ff' });
    expect(red.palette).toEqual(['#00000000', '#660000ff', '#bbcc99ff', '#222222ff']);
    expect(blue.palette).toEqual(['#00000000', '#000066ff', '#bbcc99ff', '#222222ff']);
    expect(pack.palette).toEqual(source.palette);
    expect(red.text).toEqual({
      value: '<b>Office</b>',
      color: '#fffbed',
      ...prop.customization!.text!.regions[rotation],
    });
    expect(blue.text).toBeUndefined();
    const view = render(
      <svg>
        <IndexedProp
          pack={pack}
          prop={prop}
          rotation={rotation}
          customization={{ tint: '#ff0000', text: '<b>Office</b>' }}
        />
      </svg>
    );
    expect(view.container.querySelector('text')!.textContent).toBe('<b>Office</b>');
    expect(view.container.querySelector('b, script, foreignObject')).toBeNull();
    const region = prop.customization!.text!.regions[rotation]!;
    const textClip = view.container.querySelector('text')!.parentElement!;
    expect(textClip.getAttribute('width')).toBe(String(region.width));
    expect(textClip.getAttribute('height')).toBe(String(region.height));
    expect(textClip.getAttribute('overflow')).toBe('hidden');
    view.unmount();
  }
});

it.each(['missing', 'extra', 'mixed', 'empty', 'odd', 'index', 'uppercase'])(
  'rejects invalid directional art: %s',
  (kind) => {
    const value = structuredClone(sample);
    const prop = value.props[0]!;
    if (kind === 'missing') prop.frames.pop();
    if (kind === 'extra') prop.frames.push(prop.frames[0]!);
    if (kind === 'mixed') Object.assign(prop, { pixels: ['01'] });
    if (kind === 'empty') prop.frames[0] = ['0000'];
    if (kind === 'odd') prop.frames[0] = ['001'];
    if (kind === 'index') prop.frames[0] = ['ff'];
    if (kind === 'uppercase') prop.frames[0] = ['0A'];
    expect(() => decodePropPack(value)).toThrow();
  }
);
