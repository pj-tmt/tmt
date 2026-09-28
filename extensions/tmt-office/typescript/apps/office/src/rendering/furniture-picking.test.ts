import { expect, it } from 'vitest';
import { builtinFurniture } from '../blocks/block-contract.js';
import { furnitureAt } from './furniture-picking.js';

it('picks the last painted piece when furniture overlaps', () => {
  const objects = [builtinFurniture('rug', 4, 4, 0), builtinFurniture('chair', 5, 5, 0)];
  expect(furnitureAt(objects, 5, 5)).toBe(1);
  expect(furnitureAt(objects, 4, 4)).toBe(0);
  expect(furnitureAt(objects, 10, 8)).toBeUndefined();
});

it('uses rotated footprints and excludes the far edge', () => {
  const objects = [builtinFurniture('desk', 2, 3, 1)];
  expect(furnitureAt(objects, 3, 6)).toBe(0);
  expect(furnitureAt(objects, 4, 4)).toBeUndefined();
  expect(furnitureAt(objects, 3, 7)).toBeUndefined();
  expect(furnitureAt([], 0, 0)).toBeUndefined();
});
