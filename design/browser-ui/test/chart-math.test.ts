import { expect, it } from 'vite-plus/test';
import {
  areaPath,
  compactCount,
  LEVELS,
  level,
  linePath,
  niceMax,
  stackLayers,
  ticks,
  topLayers,
} from '../src/chart-math';

const layers = [
  { key: 'a', label: 'a', values: [10, 0, null] },
  { key: 'b', label: 'b', values: [30, 0, 5] },
];

it('stacks bottom to top and keeps a missing observation apart from a zero', () => {
  const stack = stackLayers(layers);
  expect(stack.layers.map((layer) => layer.upper)).toEqual([
    [10, 0, 0],
    [40, 0, 5],
  ]);
  expect(stack.layers[1]!.lower).toEqual([10, 0, 0]);
  expect(stack.layers[0]!.missing).toEqual([false, false, true]);
  expect(stack.max).toBe(40);
});

it('share mode sums every column to 100 and leaves a silent column empty', () => {
  const stack = stackLayers(layers, { share: true });
  expect(stack.layers[1]!.upper[0]).toBe(100);
  expect(stack.layers[0]!.upper[0]).toBe(25);
  expect(stack.layers[1]!.upper[1]).toBe(0);
  expect(stack.layers[1]!.upper[2]).toBe(100);
  expect(stack.max).toBe(100);
});

it('keeps the largest layers and folds the rest into one other layer', () => {
  const many = ['a', 'b', 'c', 'd'].map((key, at) => ({
    key,
    label: key,
    values: [(at + 1) * 2, at === 3 ? null : 1],
  }));
  const kept = topLayers(many, 2);
  expect(kept.map((layer) => layer.key)).toEqual(['d', 'c', 'other']);
  expect(kept[2]!.values).toEqual([6, 2]);
  expect(topLayers(many, 4).map((layer) => layer.key)).toEqual(['d', 'c', 'b', 'a']);
  // A column where every folded layer is missing stays missing.
  const gap = topLayers(
    [
      { key: 'x', label: 'x', values: [9, 9] },
      { key: 'y', label: 'y', values: [1, null] },
    ],
    1,
  );
  expect(gap[1]!.values).toEqual([1, null]);
});

it('rounds the top of an axis to a readable step', () => {
  expect([0, -3, Number.NaN].map(niceMax)).toEqual([1, 1, 1]);
  expect([1, 1.2, 2.2, 4.9, 7, 94, 100, 101].map(niceMax)).toEqual([
    1, 2, 2.5, 5, 10, 100, 100, 200,
  ]);
  expect(ticks(100, 4)).toEqual([0, 25, 50, 75, 100]);
});

it('formats counts compactly', () => {
  expect([1_918_900_000, 48_200_000, 310_400, 940, 0].map(compactCount)).toEqual([
    '1.9B',
    '48.2M',
    '310K',
    '940',
    '0',
  ]);
});

it('levels: missing stays null, zero stays zero, one busy pair does not hide the rest', () => {
  expect(level(null, 10)).toBeNull();
  expect(level(undefined, 10)).toBeNull();
  expect(level(0, 10)).toBe(0);
  expect(level(5, 0)).toBe(0);
  expect(level(1, 1000)).toBe(1);
  expect(level(1000, 1000)).toBe(LEVELS);
  expect(level(250, 1000)).toBe(3);
  const seen = [1, 10, 100, 500, 1000].map((value) => level(value, 1000));
  expect(seen).toEqual([...seen].sort((a, b) => a! - b!));
});

it('writes path data with rounded coordinates', () => {
  expect(
    linePath([
      [0, 1.234],
      [10, 5],
    ]),
  ).toBe('M0 1.23L10 5');
  expect(areaPath([0, 10], [0, 1], [5, 6])).toBe('M0 0L10 1L10 6L0 5Z');
  expect(areaPath([], [], [])).toBe('');
});
