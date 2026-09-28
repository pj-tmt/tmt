import { expect, it } from 'vitest';
import {
  officeWorldFixture,
  WORLD_LOBBY_ID,
} from '../../../../../../../typescript/test/support/office-world.js';
import { BUILTIN_CATALOG, WALL_DIGEST, MODULAR_MOUNTED_DIGEST } from '../props/prop-contract.js';
import {
  createCatalogObject,
  suggestWallPlacement,
  hasObjectSupport,
  isWallCatalog,
  placementProblem,
} from './world-object-placement.js';
import { projectMap } from './map-geometry.js';
import type { WorldDocument, WorldObject } from './world-contract.js';
import { platformModuleWorld } from './module-upgrade.js';

const pack = BUILTIN_CATALOG.find((entry) => entry.digest === WALL_DIGEST)!;
const id = (index: number) => `30000000-0000-4000-8000-${String(index).padStart(12, '0')}`;

it('rejects unsupported, doorway and partition-crossing drops without banning floor layering', () => {
  const world = officeWorldFixture().layout;
  const object: WorldObject = {
    ...world.objects[0]!,
    kind: 'decoration',
    surface: { type: 'floor' },
    placement: {
      ...world.objects[0]!.placement,
      x: 1,
      y: 1,
      rotation: 0,
      footprint: { width: 2, height: 2 },
    },
  };
  const source = {
    version: 1 as const,
    primaryLobbyId: WORLD_LOBBY_ID,
    areas: [{ id: WORLD_LOBBY_ID, name: 'Lobby', binding: { type: 'lobby' as const } }],
    floor: Array.from({ length: 5 }, (_, y) => ({ y, start: 0, end: 6, areaId: WORLD_LOBBY_ID })),
    doors: [],
  };
  const geometry = projectMap(source);
  expect(placementProblem(geometry, object, [{ ...object, id: id(99) }])).toBeUndefined();
  const at = (x: number, y: number) => ({ ...object, placement: { ...object.placement, x, y } });
  expect(placementProblem(geometry, at(-1, 1), [])).toContain('entire object');
  expect(placementProblem(geometry, at(5, 1), [])).toContain('entire object');
  const partitioned = projectMap({
    ...source,
    areas: [
      ...source.areas,
      { id: id(9), name: 'Office', binding: { type: 'personal', identityId: null } },
    ],
    floor: source.floor.flatMap((row) => [
      { ...row, end: 3 },
      { ...row, start: 3, areaId: id(9) },
    ]),
  });
  expect(placementProblem(partitioned, at(2, 1), [])).toContain('boundary');
  const open = projectMap({
    ...source,
    areas: [
      ...source.areas,
      { id: id(9), name: 'Office', binding: { type: 'personal', identityId: null } },
    ],
    floor: source.floor.flatMap((row) => [
      { ...row, end: 3 },
      { ...row, start: 3, areaId: id(9) },
    ]),
    doors: [{ x: 3, y: 1, axis: 'vertical' }],
  });
  expect(placementProblem(open, at(1, 1), [])).toContain('entrance');
});

it('places imported wall-pack art as ordinary floor decoration on a platform', () => {
  const world = platformModuleWorld(officeWorldFixture().layout);
  const before = structuredClone(world);
  const object = createCatalogObject(world, pack, 'observatory-window', WORLD_LOBBY_ID, id(2));
  expect(object).toMatchObject({
    id: id(2),
    kind: 'decoration',
    surface: { type: 'floor' },
    extension: null,
    placement: { prop: `${pack.digest}/observatory-window` },
  });
  expect(hasObjectSupport(projectMap(world.map), object)).toBe(true);
  expect(world).toEqual(before);
});

it('groups the approved mounts with walls and derives their kinds without art-owned authority', () => {
  const mounted = BUILTIN_CATALOG.find((entry) => entry.digest === MODULAR_MOUNTED_DIGEST)!;
  expect(isWallCatalog(mounted.digest)).toBe(true);
  expect(isWallCatalog(pack.digest)).toBe(true);
  expect(isWallCatalog(BUILTIN_CATALOG[0]!.digest)).toBe(false);
  const world = officeWorldFixture().layout;
  const original = structuredClone(world);
  for (const [key, kind, height] of [
    ['mounted-window', 'window', 16],
    ['mounted-sconce', 'wallLight', 6],
    ['mounted-frame', 'decoration', 8],
    ['mounted-shelf', 'decoration', 10],
  ] as const) {
    const object = createCatalogObject(world, mounted, key, WORLD_LOBBY_ID, id(2));
    expect(object).toMatchObject({
      kind,
      extension: null,
      placement: { prop: `${mounted.digest}/${key}`, footprint: { width: height, height } },
      surface: {
        type: 'wall',
        axis: 'horizontal',
        face: 'positive',
        elevation: Math.min(3, 16 - height),
      },
    });
  }
  expect(world).toEqual(original);
});

it('checks the entire footprint and the mounted interior face, not only an object anchor', () => {
  const world = officeWorldFixture().layout;
  const geometry = projectMap(world.map);
  const floor = world.objects[0]!;
  expect(hasObjectSupport(geometry, floor)).toBe(true);
  expect(hasObjectSupport(geometry, { ...floor, placement: { ...floor.placement, x: 35 } })).toBe(
    false
  );
  const window = createCatalogObject(world, pack, 'observatory-window', WORLD_LOBBY_ID, id(2));
  expect(hasObjectSupport(geometry, window)).toBe(true);
  expect(
    hasObjectSupport(geometry, {
      ...window,
      surface: { type: 'wall', axis: 'horizontal', face: 'negative', elevation: 3 },
    })
  ).toBe(false);
  expect(hasObjectSupport(geometry, { ...window, placement: { ...window.placement, x: 35 } })).toBe(
    false
  );
});

it('adds ordinary catalog references on distinct supported wall positions without changing the map or resources', () => {
  let world = officeWorldFixture().layout;
  const map = structuredClone(world.map);
  const expected = [
    ['observatory-window', 'window', 0],
    ['brass-wall-lamp', 'wallLight', 12],
    ['orbit-poster', 'decoration', 15],
    ['crew-sign', 'decoration', 21],
    ['link-plaque', 'decoration', 29],
  ] as const;
  for (const [index, [key, kind, x]] of expected.entries()) {
    const object = createCatalogObject(world, pack, key, WORLD_LOBBY_ID, id(index + 2));
    expect(object).toMatchObject({
      kind,
      extension: null,
      placement: { prop: `${WALL_DIGEST}/${key}`, x, y: 0 },
      surface: { type: 'wall', axis: 'horizontal', face: 'positive', elevation: 3 },
    });
    world = { ...world, objects: [...world.objects, object] };
  }
  expect(world.map).toEqual(map);
  expect(world.objects[0]).toEqual(officeWorldFixture().layout.objects[0]);
});

it('uses signed vertical boundaries when the horizontal walls are too short and preserves resource bindings', () => {
  const original = officeWorldFixture().layout;
  const world: WorldDocument = {
    ...original,
    objects: [],
    map: {
      ...original.map,
      floor: Array.from({ length: 20 }, (_, index) => ({
        y: index - 10,
        start: -2,
        end: 0,
        areaId: WORLD_LOBBY_ID,
      })),
    },
  };
  const first = createCatalogObject(world, pack, 'observatory-window', WORLD_LOBBY_ID, id(2));
  expect(first).toMatchObject({
    placement: { x: -2, y: -10 },
    surface: { type: 'wall', axis: 'vertical', face: 'positive', elevation: 3 },
  });
  const linked: WorldObject = {
    ...first,
    id: id(3),
    extension: {
      definition: 'tmt-link',
      binding: { kind: 'external-link', url: 'https://example.com/' },
    },
  };
  const second = suggestWallPlacement({ ...world, objects: [first] }, linked, WORLD_LOBBY_ID);
  expect(second).toMatchObject({
    placement: { x: 0, y: -10 },
    surface: { type: 'wall', axis: 'vertical', face: 'negative', elevation: 3 },
    extension: linked.extension,
  });
});

it('never silently falls back to floor or another area when the requested wall cannot fit', () => {
  const original = officeWorldFixture().layout;
  const tiny: WorldDocument = {
    ...original,
    map: { ...original.map, floor: [{ y: 0, start: 0, end: 2, areaId: WORLD_LOBBY_ID }] },
  };
  expect(() =>
    createCatalogObject(tiny, pack, 'observatory-window', WORLD_LOBBY_ID, id(2))
  ).toThrow('No suitable wall');
  expect(() => createCatalogObject(tiny, pack, 'orbit-poster', null, id(2))).toThrow(
    'Choose an area with floor before adding an object.'
  );
  expect(tiny.objects).toEqual(original.objects);
});
