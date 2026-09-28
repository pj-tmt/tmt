import { expect, it } from 'vitest';
import { defaultCatalog, validLayout } from './block-contract.js';
import { resolvePlacedProp } from '../props/prop-contract.js';
import { WORKSHOP_STARTERS, workshopStarter } from './workshop-starter.js';

it.each(WORKSHOP_STARTERS)(
  '$label keeps the working desk within its recipe footprint',
  (recipe) => {
    const layout = recipe.create();
    const desk = layout.find((item) => item.prop.endsWith('/oak-desk'))!;
    expect(desk).toBeDefined();
    expect(desk.y + desk.footprint.height).toBeLessThanOrEqual(32);
  }
);

it.each(WORKSHOP_STARTERS)(
  '$label is admitted, serializable and leaves an open staging corner',
  (recipe) => {
    const layout = recipe.create();
    expect(validLayout(layout)).toBe(true);
    expect(JSON.parse(JSON.stringify(layout))).toEqual(layout);
    for (const item of layout) {
      expect(resolvePlacedProp(defaultCatalog(), item)).toBeDefined();
      if (item.prop.endsWith('/woven-rug')) continue;
      for (const clear of [{ x: 23, y: 28, width: 6, height: 4 }]) {
        const overlaps =
          item.x < clear.x + clear.width &&
          item.x + item.footprint.width > clear.x &&
          item.y < clear.y + clear.height &&
          item.y + item.footprint.height > clear.y;
        expect(overlaps, `${recipe.id}: ${item.prop} obstructs the staging corner`).toBe(false);
      }
    }
    layout[0]!.x = 31;
    expect(recipe.create()[0]!.x).not.toBe(31);
  }
);

it('offers genuinely different object arrangements, not only different recipe labels', () => {
  expect(new Set(WORKSHOP_STARTERS.map((recipe) => JSON.stringify(recipe.create()))).size).toBe(3);
});

it('uses admitted furniture in a bounded editable layout with rugs behind furniture', () => {
  const layout = workshopStarter();
  expect(validLayout(layout)).toBe(true);
  expect(layout).toHaveLength(11);
  const names = layout.map((item) => resolvePlacedProp(defaultCatalog(), item)?.definition.key);
  expect(names).toEqual([
    'woven-rug',
    'woven-rug',
    'oak-bookcase',
    'reading-lamp',
    'green-chair',
    'oak-desk',
    'lounge-sofa',
    'coffee-table',
    'leafy-plant',
    'leafy-plant',
    'desktop-terminal',
  ]);
  // The resident stands in the lower-right doorway, away from the desk and lounge.
  expect(layout.every((item) => item.y + item.footprint.height <= 19 || item.x < 10)).toBe(true);
});

it('creates independent drafts without mutating catalog footprints or later recipes', () => {
  const first = workshopStarter();
  const original = workshopStarter();
  first[0]!.x = 31;
  first[0]!.footprint.width = 1;
  first.pop();
  expect(workshopStarter()).toEqual(original);
});
