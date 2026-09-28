import { expect, it } from 'vitest';
import {
  officeWorldFixture,
  WORLD_LOBBY_ID,
} from '../../../../../../../typescript/test/support/office-world.js';
import { worldGeometry, intersects, wallProjection } from './world-geometry.js';
import { footprint } from '../blocks/block-contract.js';
import { AREA_ACTOR_PREVIEW_LIMIT, areaActorSlots } from './actor-slots.js';
import central from '../../../../../../../contracts/office/modules-central-grid-vectors.json';
import { decodeModuleMap } from '../world-map/module-contract.js';

it('prefers a clear view over the area center without changing full-height wall geometry', () => {
  const world = { ...officeWorldFixture().layout, map: decodeModuleMap(central.map), objects: [] };
  const original = structuredClone(world);
  const geometry = worldGeometry(world);
  const area = geometry.map.areaAt(24, -28)!;
  const slot = geometry.actorSlots(area)[0]!;
  expect(slot).toBeDefined();
  const actor = geometry.projection.projectUpright(slot);
  const walls = geometry.visible(geometry.bounds).walls;
  const hiddenBy = (rect: typeof actor) =>
    walls.filter((wall) => {
      const painted = wallProjection(wall, geometry.projection);
      return painted.depth >= rect.y + rect.height && intersects(painted.bounds, rect);
    });
  // Positive control outside the central doorway lies behind the solid front wall.
  const center = geometry.projection.projectUpright({ ...slot, x: 8, y: -20 });
  expect(hiddenBy(center).length).toBeGreaterThan(0);
  const centerFirst = areaActorSlots(
    Array.from({ length: 40 }, (_, offset) => ({
      y: -48 + offset,
      start: 0,
      end: 48,
      areaId: area,
    })),
    { x: 24, y: -27.5 },
    []
  )[0]!;
  expect(hiddenBy(geometry.projection.projectUpright(centerFirst)).length).toBeGreaterThan(0);
  expect(hiddenBy(actor)).toEqual([]);
  expect(world).toEqual(original);
});

it('retains safe fallback slots when no candidate has a clear view', () => {
  const rows = Array.from({ length: 40 }, (_, y) => ({
    y,
    start: 0,
    end: 40,
    areaId: WORLD_LOBBY_ID,
  }));
  const anchor = { x: 20, y: 20 };
  const obstacles = [{ x: 0, y: 0, width: 12, height: 40 }];
  const baseline = areaActorSlots(rows, anchor, obstacles);
  expect(baseline.length).toBeGreaterThan(0);
  expect(baseline[0]!.x).toBeLessThan(30);
  expect(
    areaActorSlots(rows, anchor, obstacles, (rect) => rect.x >= 30)[0]!.x
  ).toBeGreaterThanOrEqual(30);
  expect(areaActorSlots(rows, anchor, obstacles, () => false)).toEqual(baseline);
  expect(areaActorSlots(rows, anchor, obstacles, () => true)).toEqual(baseline);
});

it('caches bounded slots fully on the area floor and away from furniture', () => {
  const initial = officeWorldFixture().layout;
  const world = {
    ...initial,
    map: {
      ...initial.map,
      floor: Array.from({ length: 48 }, (_, y) => ({
        y,
        start: 0,
        end: 48,
        areaId: WORLD_LOBBY_ID,
      })),
    },
  };
  const geometry = worldGeometry(world);
  const slots = geometry.actorSlots(WORLD_LOBBY_ID);
  expect(slots).toHaveLength(AREA_ACTOR_PREVIEW_LIMIT);
  expect(geometry.actorSlots(WORLD_LOBBY_ID)).toBe(slots);
  for (const rect of slots) {
    for (let y = Math.floor(rect.y); y < Math.ceil(rect.y + rect.height); y++)
      for (let x = Math.floor(rect.x); x < Math.ceil(rect.x + rect.width); x++)
        expect(geometry.map.areaAt(x, y)).toBe(WORLD_LOBBY_ID);
    expect(slots.filter((other) => intersects(rect, other))).toEqual([rect]);
    expect(
      world.objects.some((object) =>
        intersects(rect, {
          ...object.placement,
          ...footprint(object.placement),
        })
      )
    ).toBe(false);
  }
});

it('does not force previews across narrow or concave boundaries, and never borrows common floor', () => {
  const initial = officeWorldFixture().layout;
  const world = {
    ...initial,
    objects: [],
    map: {
      ...initial.map,
      floor: [
        ...Array.from({ length: 40 }, (_, y) => ({
          y,
          start: 0,
          end: y < 14 ? 22 : 5,
          areaId: WORLD_LOBBY_ID,
        })),
        ...Array.from({ length: 26 }, (_, dy) => ({ y: dy + 14, start: 5, end: 22, areaId: null })),
      ],
    },
  };
  const geometry = worldGeometry(world);
  const slots = geometry.actorSlots(WORLD_LOBBY_ID);
  expect(slots.length).toBeGreaterThan(0);
  expect(slots.length).toBeLessThan(AREA_ACTOR_PREVIEW_LIMIT);
  expect(slots.every((rect) => rect.y + rect.height < 14)).toBe(true);
  const narrow = {
    ...world,
    map: { ...world.map, floor: [{ y: 0, start: 0, end: 100, areaId: WORLD_LOBBY_ID }] },
  };
  expect(worldGeometry(narrow).actorSlots(WORLD_LOBBY_ID)).toEqual([]);
  expect(geometry.actorSlots('not-an-area')).toEqual([]);
});

it('uses deterministic positions independent of row segmentation and area-list order', () => {
  const world = officeWorldFixture().layout;
  const expected = worldGeometry(world).actorSlots(WORLD_LOBBY_ID);
  const split = {
    ...world,
    map: {
      ...world.map,
      floor: world.map.floor
        .flatMap((row) => [
          { ...row, end: 18 },
          { ...row, start: 18 },
        ])
        .reverse(),
    },
  };
  // The canonical area anchor must also be independent of how equal floor runs are encoded.
  expect(worldGeometry(split).actorSlots(WORLD_LOBBY_ID)).toEqual(expected);
});
