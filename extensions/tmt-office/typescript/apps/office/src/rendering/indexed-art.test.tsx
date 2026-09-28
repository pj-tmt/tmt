import { render } from '@testing-library/react';
import { expect, it } from 'vitest';
import { indexedRuns, tintPalette } from './indexed-art.js';
import { IndexedRaster } from './indexed-raster.js';

const palette = ['#00000000', '#112233ff', '#aabbccff'];

it('tints only admitted material slots, preserving brightness and the source palette', () => {
  const source = ['#00000000', '#808080ff', '#ffffffff', '#112233ff'];
  expect(tintPalette(source, [1, 2], '#804020')).toEqual([
    '#00000000',
    '#402010ff',
    '#804020ff',
    '#112233ff',
  ]);
  expect(source).toEqual(['#00000000', '#808080ff', '#ffffffff', '#112233ff']);
});

it('merges horizontal runs without crossing transparency, colors, or rows', () => {
  expect(Array.from(indexedRuns({ pixels: ['11012', '11100'], palette }))).toEqual([
    { x: 0, y: 0, width: 2, color: palette[1] },
    { x: 3, y: 0, width: 1, color: palette[1] },
    { x: 4, y: 0, width: 1, color: palette[2] },
    { x: 0, y: 1, width: 3, color: palette[1] },
  ]);
  expect(
    Array.from(indexedRuns({ pixels: ['0101000102', '0101010000'], palette, indexWidth: 2 }))
  ).toEqual(Array.from(indexedRuns({ pixels: ['11012', '11100'], palette })));
});

it('renders exact inert geometry grouped by color instead of a DOM node per cell', () => {
  const view = render(
    <IndexedRaster pixels={['11012', '11100']} palette={palette} width={5} height={2} />
  );
  const paths = Array.from(view.container.querySelectorAll('path'));
  expect(paths.map((path) => [path.getAttribute('fill'), path.getAttribute('d')])).toEqual([
    [palette[1], 'M0 0h2v1h-2zM3 0h1v1h-1zM0 1h3v1h-3z'],
    [palette[2], 'M4 0h1v1h-1z'],
  ]);
  expect(view.container.querySelector('script, image, foreignObject, rect')).toBeNull();
});

it('bounds DOM nodes even for a full-size alternating directional frame', () => {
  const pixels = Array.from({ length: 128 }, (_, row) => (row % 2 ? '0201' : '0102').repeat(64));
  const view = render(
    <IndexedRaster pixels={pixels} palette={palette} indexWidth={2} width={8} height={8} />
  );
  expect(view.container.querySelector('svg')?.getAttribute('viewBox')).toBe('0 0 128 128');
  const paths = Array.from(view.container.querySelectorAll('path'));
  expect(paths).toHaveLength(2);
  expect(paths.map((path) => path.getAttribute('d')!.match(/M/g)!.length)).toEqual([8192, 8192]);
});
