import { expect, it } from 'vite-plus/test';
import {
  officeWorldFixture,
  WORLD_LOBBY_ID,
} from '../../../../../../../typescript/test/support/office-world.js';
import {
  worldGeometry,
  worldObjectRect,
  worldObjectPosition,
  wallProjection,
} from './world-geometry.js';
import { updateWorldMap } from '../world-map/world-draft.js';
import vectors from '../../../../../contracts/modules-v2-vectors.json';
import { decodeModuleMap } from '../world-map/module-contract.js';
import central from '../../../../../contracts/modules-central-grid-vectors.json';
import { moduleGhostGeometry } from './scene-module-ghost.js';
import { mapGeometry } from '../world-map/map-source.js';
import islands from '../../../../../contracts/modules-island-vectors.json';

it('includes the complete independent wing in Fit beside a wider northern campus', () => {
  const source = decodeModuleMap(islands.map);
  const office = source.modules.find((module) => module.slot.type === 'office')!;
  const meeting = source.modules.find((module) => module.slot.type === 'meeting')!;
  const geometry = worldGeometry({
    version: 1,
    objects: [],
    map: {
      ...source,
      modules: [
        ...source.modules,
        ...[1, 2, 3, 4].map((column) => ({
          ...office,
          area: { ...office.area, id: `30000000-0000-4000-8000-00000000000${column}` },
          slot: { type: 'office' as const, column, row: -1 },
        })),
        {
          ...meeting,
          area: {
            ...meeting.area,
            id: '30000000-0000-4000-8000-000000000005',
            binding: { type: 'meeting', roomId: '40000000-0000-4000-8000-000000000005' },
          },
          slot: { type: 'meeting', index: 4 },
        },
      ],
    },
  });
  expect(geometry.bounds.y + geometry.bounds.height).toBe(267);
  const floor = geometry
    .visible(geometry.bounds)
    .floors.filter((rect) => rect.areaId === '30000000-0000-4000-8000-000000000005');
  expect(Math.max(...floor.map((rect) => rect.y + rect.height))).toBe(259);
});

it('indexes visible platform thresholds on both axes without restoring tall walls', () => {
  const geometry = worldGeometry({
    ...officeWorldFixture().layout,
    map: decodeModuleMap({ ...central.map, version: 6 }),
    objects: [],
  });
  const walls = geometry.visible(geometry.bounds).walls;
  const doors = walls.filter((wall) => wall.open);
  expect(doors.length).toBeGreaterThan(0);
  expect(new Set(doors.map((door) => door.axis))).toEqual(new Set(['horizontal', 'vertical']));
  for (const door of doors) {
    const bounds = wallProjection(door, geometry.projection).bounds;
    expect(bounds.width).toBeGreaterThan(0);
    expect(bounds.height).toBeGreaterThan(0);
  }
  expect(geometry.map.boundaries.some((boundary) => boundary.open)).toBe(true);
  const solid = walls.find(
    (wall) => !wall.open && !wall.circulation && wall.axis === 'horizontal'
  )!;
  expect(wallProjection(solid, geometry.projection).bounds.height).toBeLessThanOrEqual(6);
});

it('fills platform floors to their flush edge without changing public decking', () => {
  const world = {
    ...officeWorldFixture().layout,
    map: decodeModuleMap({ ...central.map, version: 6 }),
    objects: [],
  };
  const geometry = worldGeometry(world);
  const floor = { x: 0, y: 16, width: 104, height: 61, areaId: world.map.primaryLobbyId };
  expect(geometry.floorPaintBounds(floor)).toEqual({
    x: 0,
    y: 16,
    width: 104,
    height: 61,
    areaId: floor.areaId,
  });
  expect(floor).toMatchObject({ x: 0, y: 16, width: 104, height: 61 });
  const bridge = { x: 20, y: -7, width: 8, height: 7, areaId: null };
  expect(geometry.floorPaintBounds(bridge)).toBe(bridge);
});

it('projects platform and public coordinates onto one plane without wall reserves', () => {
  const geometry = worldGeometry({
    ...officeWorldFixture().layout,
    map: decodeModuleMap({ ...central.map, version: 6 }),
    objects: [],
  });
  for (const point of [
    { x: 20, y: 0 },
    { x: 20, y: 88 },
    { x: 0, y: 20 },
  ]) {
    const floor = geometry.projection.projectGround(point, geometry.map.areaAt(point.x, point.y));
    expect(floor).toEqual(geometry.projection.projectGround(point, null));
    expect(floor.y).toBe((point.y * 7) / 8);
  }
});

it.each([4, 5, 6])(
  'places modular nameplates on rear wall headers without moving floor or actor anchors (v%s)',
  (version) => {
    const world = {
      ...officeWorldFixture().layout,
      map: decodeModuleMap({ ...central.map, version }),
      objects: [],
    };
    const original = structuredClone(world);
    const geometry = worldGeometry(world);
    const byName = new Map(mapGeometry(world.map).areas.map((area) => [area.name, area.id]));
    expect(geometry.nameplate(byName.get('Lobby')!)).toEqual({ x: 52, y: version >= 6 ? 3 : 6 });
    expect(geometry.nameplate(byName.get('North west')!)).toEqual({
      x: 24,
      y: version >= 6 ? -53 : version >= 5 ? -36 : -42,
    });
    expect(geometry.nameplate(byName.get('North east')!)).toEqual({
      x: 80,
      y: version >= 6 ? -53 : version >= 5 ? -36 : -42,
    });
    expect(geometry.nameplate(byName.get('South west')!)).toEqual({
      x: 24,
      y: version >= 6 ? 101 : version >= 5 ? 90 : 102,
    });
    expect(geometry.nameplate('missing-area')).toBeUndefined();
    expect(geometry.anchors.get(byName.get('Lobby')!)?.y).toBe(44.5);
    expect(world).toEqual(original);
  }
);

it('adds a side crown only at the physical endpoint, never again after a portal split', () => {
  const geometry = worldGeometry({
    ...officeWorldFixture().layout,
    map: decodeModuleMap(central.map),
    objects: [],
  });
  const sides = geometry.visible(geometry.bounds).walls.filter((wall) => wall.axis === 'vertical');
  const followers = sides.filter((wall) =>
    sides.some(
      (other) =>
        other.areaId === wall.areaId && other.x === wall.x && other.y + other.height === wall.y
    )
  );
  expect(followers.length).toBeGreaterThan(0);
  expect(followers.every((wall) => wall.sideStart === false)).toBe(true);
  expect(sides.some((wall) => wall.sideStart === true && !wall.open)).toBe(true);
});

it('emits each central-grid wall edge once while retaining both sides of a real corridor', () => {
  const world = { ...officeWorldFixture().layout, map: decodeModuleMap(central.map), objects: [] };
  const geometry = worldGeometry(world);
  const walls = geometry.visible(geometry.bounds).walls;
  const edges: string[] = [];
  for (const wall of walls) {
    const horizontal = wall.axis === 'horizontal';
    for (let offset = 0; offset < (horizontal ? wall.width : wall.height); offset++)
      edges.push(
        `${wall.axis}:${wall.x + (horizontal ? offset : 0)}:${wall.y + (horizontal ? 0 : offset)}`
      );
  }
  expect(new Set(edges).size).toBe(edges.length);
  // These are distinct physical boundaries separated by eight tiles of passage,
  // not duplicate front/back renderings of one boundary.
  expect(edges).toContain('horizontal:10:-8');
  expect(edges).toContain('horizontal:10:0');
});

it('keeps the full horizontal passage visible between equally tall opposing room walls', () => {
  const world = { ...officeWorldFixture().layout, map: decodeModuleMap(central.map), objects: [] };
  const geometry = worldGeometry(world);
  const walls = geometry.visible(geometry.bounds).walls;
  // Check both sides of the lobby, away from doors: a portal alone must not
  // conceal a wall projection that covers the rest of the public passage.
  for (const [north, south] of [
    [-8, 0],
    [88, 96],
  ] as const) {
    const boundaries = [north, south].map((y) => {
      const wall = walls.find(
        (candidate) =>
          candidate.axis === 'horizontal' &&
          candidate.y === y &&
          candidate.x <= 10 &&
          candidate.x + candidate.width > 10 &&
          !candidate.open
      );
      expect(wall).toBeDefined();
      return wallProjection(wall!, geometry.projection).bounds;
    });
    const [front, rear] = boundaries;
    const passage = geometry.projection.projectGroundRect(
      { x: 10, y: north, width: 1, height: south - north },
      null
    );
    expect(front!.height).toBe(16);
    expect(rear!.height).toBe(front!.height);
    expect(front!.y + front!.height).toBe(passage.y);
    expect(rear!.y).toBe(passage.y + passage.height);
    expect(passage.height).toBe(8);
  }
});

it('owns front corner posts only at side returns, never alongside door jambs', () => {
  const world = { ...officeWorldFixture().layout, map: decodeModuleMap(central.map), objects: [] };
  const original = structuredClone(world);
  const geometry = worldGeometry(world);
  const walls = geometry.visible(geometry.bounds).walls;
  const front = walls.filter(
    (wall) => wall.axis === 'horizontal' && wall.y === -8 && wall.x >= 0 && wall.x < 48
  );
  expect(front.map((wall) => [wall.x, wall.width, wall.open, wall.frontCorners])).toEqual([
    [0, 20, false, { start: true, end: false }],
    [20, 8, true, undefined],
    [28, 20, false, { start: false, end: true }],
  ]);
  for (const wall of front.filter((wall) => !wall.open))
    expect(wallProjection(wall, geometry.projection).bounds.height).toBe(16);
  expect(wallProjection(front[1]!, geometry.projection).bounds.height).toBe(16);
  expect(world).toEqual(original);
});

it('reserves rear walls inside central-grid cells without widening roomless circulation', () => {
  const world = { ...officeWorldFixture().layout, map: decodeModuleMap(central.map), objects: [] };
  const original = structuredClone(world);
  const geometry = worldGeometry(world);
  const { projection } = geometry;
  const walls = geometry.visible(geometry.bounds).walls;
  const rear = walls.find((wall) => wall.axis === 'horizontal' && wall.x === 0 && wall.y === -48)!;
  const rearArt = wallProjection(rear, projection);
  expect(rearArt.bounds).toEqual({ x: 0, y: -48, width: 48, height: 16 });
  expect(projection.projectGroundRect({ x: -100, y: 40, width: 80, height: 8 }, null)).toEqual({
    x: -100,
    y: 40,
    width: 80,
    height: 8,
  });
  expect(projection.projectGroundRect({ x: 48, y: -40, width: 8, height: 24 }, null)).toEqual({
    x: 48,
    y: -40,
    width: 8,
    height: 24,
  });
  const west = walls.find((wall) => wall.axis === 'vertical' && wall.x === 48 && wall.y === -48)!;
  const east = walls.find((wall) => wall.axis === 'vertical' && wall.x === 56 && wall.y === -48)!;
  const a = wallProjection(west, projection).bounds;
  const b = wallProjection(east, projection).bounds;
  expect(b.x - (a.x + a.width)).toBe(8);
  const ghost = moduleGhostGeometry({ type: 'office', column: 0, row: -1 }, projection);
  expect(ghost.floor).toEqual({ x: 0, y: -32, width: 48, height: 24 });
  expect(ghost.bounds).toEqual({ x: 0, y: -48, width: 48, height: 40 });
  expect(ghost.bounds.y).toBe(rearArt.bounds.y);
  for (const point of [
    { x: 12, y: -47.5 },
    { x: 12, y: -20 },
    { x: 50, y: 44 },
    { x: -100, y: 44 },
    { x: 116, y: 160 },
    { x: 130, y: 150 },
  ]) {
    const restored = projection.unprojectGround(projection.projectGround(point));
    expect(restored.x).toBe(point.x);
    expect(restored.y).toBeCloseTo(point.y, 10);
  }
  expect(world).toEqual(original);
});

it.each([4, 5])(
  'retains mounted and floor object positions across the cell-owned wall reserve (v%s)',
  (version) => {
    const fixture = officeWorldFixture().layout;
    const floor = {
      ...fixture.objects[0]!,
      placement: { ...fixture.objects[0]!.placement, x: 8, y: -30 },
    };
    const back = {
      ...floor,
      id: 'back',
      surface: {
        type: 'wall' as const,
        axis: 'horizontal' as const,
        face: 'positive' as const,
        elevation: 3,
      },
      placement: { ...floor.placement, y: -48 },
    };
    const side = {
      ...back,
      id: 'side',
      surface: { ...back.surface, axis: 'vertical' as const, face: 'negative' as const },
      placement: { ...back.placement, x: 48, y: -30 },
    };
    const geometry = worldGeometry({
      ...fixture,
      map: decodeModuleMap({ ...central.map, version }),
      objects: [floor, back, side],
    });
    for (const object of [floor, back, side]) {
      expect(geometry.objectPosition(object, geometry.objectRect(object))).toEqual({
        x: object.placement.x,
        y: object.placement.y,
      });
      const moved = {
        ...object,
        placement: { ...object.placement, ...(object === back ? { x: 16 } : { y: -24 }) },
      };
      expect(geometry.objectPosition(object, geometry.objectRect(moved))).toEqual({
        x: moved.placement.x,
        y: moved.placement.y,
      });
    }
    expect(geometry.objectDepth(floor)).toBeGreaterThan(-32);
    expect(geometry.objectDepth(floor)).toBeLessThan(-8);
  }
);

it.each([4, 5])(
  'uses the declared wall face for room-side and corridor-side mount inversion (v%s)',
  (version) => {
    const fixture = officeWorldFixture().layout;
    const inside = {
      ...fixture.objects[0]!,
      surface: {
        type: 'wall' as const,
        axis: 'horizontal' as const,
        face: 'positive' as const,
        elevation: 2,
      },
      placement: { ...fixture.objects[0]!.placement, x: 8, y: 96 },
    };
    const outside = {
      ...inside,
      id: 'outside',
      surface: { ...inside.surface, face: 'negative' as const },
    };
    const geometry = worldGeometry({
      ...fixture,
      map: decodeModuleMap({ ...central.map, version }),
      objects: [inside, outside],
    });
    expect(geometry.objectRect(inside).y - geometry.objectRect(outside).y).toBe(16);
    for (const object of [inside, outside]) {
      expect(geometry.objectPosition(object, geometry.objectRect(object))).toEqual({ x: 8, y: 96 });
      const moved = { ...object, placement: { ...object.placement, x: 12 } };
      expect(geometry.objectPosition(object, geometry.objectRect(moved))).toEqual({ x: 12, y: 96 });
    }
  }
);

it('keeps visible common floor between a cutaway front sill and the next full-height back wall', () => {
  const source = {
    ...officeWorldFixture().layout,
    map: decodeModuleMap({ ...vectors.starter, version: 3 }),
  };
  const geometry = worldGeometry(source);
  const walls = geometry.visible(geometry.bounds).walls;
  const front = walls.find((wall) => wall.axis === 'horizontal' && wall.x === 0 && wall.y === -8)!;
  const back = walls.find((wall) => wall.axis === 'horizontal' && wall.x === 0 && wall.y === 0)!;
  expect(front.raised).toBe(false);
  expect(back.raised).toBe(true);
  const a = wallProjection(front, geometry.projection),
    b = wallProjection(back, geometry.projection);
  expect(a.art).toBe('sill');
  expect(b.art).toBe('back');
  expect(b.bounds.height).toBe(16);
  expect(b.bounds.y - (a.bounds.y + a.bounds.height)).toBe(4);
  const west = walls.find((wall) => wall.axis === 'vertical' && wall.x === 48 && wall.y === -48)!;
  const east = walls.find((wall) => wall.axis === 'vertical' && wall.x === 56 && wall.y === -48)!;
  const westArt = wallProjection(west, geometry.projection),
    eastArt = wallProjection(east, geometry.projection);
  expect(westArt.art).toBe('right');
  expect(eastArt.art).toBe('left');
  expect(eastArt.bounds.x - (westArt.bounds.x + westArt.bounds.width)).toBe(4);
  expect(geometry.map.areaAt(50, -40)).toBeNull();
  expect(geometry.map.areaAt(1, -4)).toBeNull();
  const object = {
    ...source.objects[0]!,
    placement: { ...source.objects[0]!.placement, x: 4, y: -20 },
  };
  const before = geometry.objectRect(object);
  const after = geometry.objectRect({ ...object, placement: { ...object.placement, y: 10 } });
  expect(geometry.objectPosition(object, after)).toEqual({ x: 4, y: 10 });
  expect(geometry.objectPosition(object, before)).toEqual({ x: 4, y: -20 });
  expect(after.width).toBe(before.width);
  expect(after.height).toBe(before.height);
});

it('coalesces floor and boundary projections without inventing surrounding floor', () => {
  const world = officeWorldFixture().layout;
  const geometry = worldGeometry(world);
  const all = geometry.visible({ x: -20, y: -20, width: 100, height: 100 });
  expect(all.floors.reduce((sum, rect) => sum + rect.width * rect.height, 0)).toBe(810);
  expect(geometry.map.tileCount).toBe(36 * 36);
  expect(all.floors).toHaveLength(2);
  expect(all.walls).toHaveLength(4);
  expect(geometry.map.areaAt(-1, 0)).toBeUndefined();
  expect(all.objects).toEqual([0]);
});

it('derives low corridor rails from common floor rather than a second wall flag in storage', () => {
  const source = { ...officeWorldFixture().layout, map: decodeModuleMap(vectors.starter) };
  const geometry = worldGeometry(source);
  const walls = geometry.visible(geometry.bounds).walls;
  expect(walls).toContainEqual(
    expect.objectContaining({
      x: 20,
      y: -8,
      axis: 'vertical',
      circulation: true,
    })
  );
  expect(walls).toContainEqual(
    expect.objectContaining({
      x: 0,
      y: -48,
      axis: 'vertical',
      circulation: false,
    })
  );
  expect(source.map).not.toHaveProperty('walls');
});

it('paints one portal per module passage while retaining both physical openings', () => {
  const map = decodeModuleMap(vectors.starter);
  const source = { ...officeWorldFixture().layout, map };
  const geometry = worldGeometry(source);
  const portals = geometry.visible(geometry.bounds).walls.filter((wall) => wall.open);
  const positions = (walls: typeof portals) =>
    walls.map(({ x, y, axis }) => `${x},${y},${axis}`).sort();
  expect(positions(portals)).toEqual([
    '20,0,horizontal',
    '20,48,horizontal',
    '56,-32,vertical',
    '56,64,vertical',
    '76,0,horizontal',
    '76,48,horizontal',
  ]);
  expect(geometry.map.boundaryAt({ x: 20, y: -8, axis: 'horizontal' })?.open).toBe(true);
  expect(geometry.map.boundaryAt({ x: 20, y: 0, axis: 'horizontal' })?.open).toBe(true);
  expect(geometry.map.areaAt(23, -4)).toBeNull();
  expect(geometry.map.tileCount).toBe(12224);
  const reordered = worldGeometry({
    ...source,
    map: { ...map, modules: [...map.modules].reverse() },
  });
  expect(positions(reordered.visible(reordered.bounds).walls.filter((wall) => wall.open))).toEqual(
    positions(portals)
  );
  expect(source.map).not.toHaveProperty('doors');
});

it('keeps sparse remote floor and objects out of a local viewport', () => {
  const original = officeWorldFixture().layout;
  const world = {
    ...original,
    map: {
      ...original.map,
      floor: [...original.map.floor, { y: 3000, start: 3000, end: 3008, areaId: null }],
    },
    objects: [
      ...original.objects,
      {
        ...original.objects[0]!,
        id: '30000000-0000-4000-8000-000000000002',
        placement: { ...original.objects[0]!.placement, x: 3000, y: 3000 },
      },
    ],
  };
  const geometry = worldGeometry(world);
  expect(geometry.visible({ x: -1, y: -1, width: 45, height: 45 }).objects).toEqual([0]);
  const far = geometry.visible({ x: 2990, y: 1865, width: 30, height: 30 });
  expect(far.objects).toEqual([1]);
  expect(far.floors.reduce((sum, rect) => sum + rect.width * rect.height, 0)).toBe(5);
  expect(geometry.visible({ x: -9000, y: -9000, width: 10, height: 10 }).floors).toEqual([]);
});

it('uses bounded merged primitives at the full tile budget and culls offscreen chunks', () => {
  const source = officeWorldFixture().layout;
  const world = updateWorldMap(source, {
    ...source.map,
    floor: Array.from({ length: 512 }, (_, y) => ({
      y,
      start: 0,
      end: 512,
      areaId: WORLD_LOBBY_ID,
    })),
  });
  const geometry = worldGeometry(world);
  expect(geometry.map.tileCount).toBe(262144);
  const slots = geometry.actorSlots(WORLD_LOBBY_ID);
  expect(slots).toHaveLength(6);
  expect(geometry.actorSlots(WORLD_LOBBY_ID)).toBe(slots);
  const all = geometry.visible({ x: -20, y: -20, width: 560, height: 560 });
  expect(all.floors).toHaveLength(160);
  expect(all.walls).toHaveLength(4);
  const near = geometry.visible({ x: 200, y: 200, width: 40, height: 40 });
  expect(near.floors.length).toBeLessThanOrEqual(16);
  expect(near.walls).toEqual([]);
});

it('derives partition openings and keeps the area anchor on actual concave floor', () => {
  const source = officeWorldFixture().layout;
  const id = '10000000-0000-4000-8000-000000000002';
  const map = {
    ...source.map,
    areas: [
      ...source.map.areas,
      { id, name: 'Studio', binding: { type: 'personal' as const, identityId: null } },
    ],
    floor: [
      { y: 0, start: 0, end: 10, areaId: WORLD_LOBBY_ID },
      { y: 1, start: 0, end: 2, areaId: WORLD_LOBBY_ID },
      { y: 1, start: 2, end: 10, areaId: id },
    ],
    doors: [{ x: 2, y: 1, axis: 'vertical' as const }],
  };
  const geometry = worldGeometry({ ...source, map });
  const anchor = geometry.anchors.get(WORLD_LOBBY_ID)!;
  expect(geometry.map.areaAt(Math.floor(anchor.x), Math.floor(anchor.y))).toBe(WORLD_LOBBY_ID);
  const doors = geometry.visible(geometry.bounds).walls.filter((wall) => wall.open);
  expect(doors).toHaveLength(1);
  expect(doors[0]).toMatchObject({ x: 2, y: 1, axis: 'vertical', exterior: false });
});

it('projects wall mounts separately from their stored surface coordinates', () => {
  const object = officeWorldFixture().layout.objects[0]!;
  expect(worldObjectRect(object)).toEqual({ x: 4, y: 1.75, width: 4, height: 2 });
  expect(
    worldObjectRect({
      ...object,
      surface: { type: 'wall', axis: 'horizontal', face: 'positive', elevation: 3 },
    })
  ).toEqual({ x: 4, y: -2.5, width: 4, height: 2 });
  expect(object.placement.y).toBe(4);
});

it('paints wall-mounted objects after their supporting wall without changing stored elevation', () => {
  const source = officeWorldFixture().layout;
  const template = source.objects[0]!;
  const objects = [
    template,
    {
      ...template,
      id: '30000000-0000-4000-8000-000000000002',
      placement: { ...template.placement, x: 8, y: 0 },
      surface: {
        type: 'wall' as const,
        axis: 'horizontal' as const,
        face: 'positive' as const,
        elevation: 6,
      },
    },
    {
      ...template,
      id: '30000000-0000-4000-8000-000000000003',
      placement: { ...template.placement, x: 0, y: 8 },
      surface: {
        type: 'wall' as const,
        axis: 'vertical' as const,
        face: 'positive' as const,
        elevation: 4,
      },
    },
  ];
  const geometry = worldGeometry({ ...source, objects });
  expect(geometry.objectDepth(objects[0]!)).toBe(Number.NEGATIVE_INFINITY);
  expect(geometry.objectDepth(objects[1]!)).toBe(0.01);
  expect(geometry.objectDepth(objects[2]!)).toBe(22.51);
  expect(worldObjectRect(objects[1]!)).toEqual({ x: 8, y: -8, width: 4, height: 2 });
  expect(objects[1]!.placement).toMatchObject({ x: 8, y: 0 });
  expect(objects[2]!.surface).toMatchObject({ elevation: 4 });
});

it('includes maximum wall-mount extents in Fit, including the outer vertical face', () => {
  const source = officeWorldFixture().layout;
  const object = {
    ...source.objects[0]!,
    placement: { ...source.objects[0]!.placement, x: 0, footprint: { width: 4, height: 16 } },
    surface: {
      type: 'wall' as const,
      axis: 'vertical' as const,
      face: 'positive' as const,
      elevation: 0,
    },
  };
  const geometry = worldGeometry({ ...source, objects: [object] });
  const rect = worldObjectRect(object);
  expect(rect.x).toBe(-8);
  expect(geometry.bounds.x).toBeLessThan(rect.x);
  expect(geometry.bounds.x + geometry.bounds.width).toBeGreaterThan(rect.x + rect.width);
});

it('inverse-projects moves for both wall faces and rotations without an elevation jump', () => {
  const original = officeWorldFixture().layout.objects[0]!;
  for (const axis of ['horizontal', 'vertical'] as const)
    for (const face of ['positive', 'negative'] as const)
      for (const rotation of [0, 1, 2, 3]) {
        const object = {
          ...original,
          placement: { ...original.placement, x: -20, y: 30, rotation },
          surface: { type: 'wall' as const, axis, face, elevation: 7 },
        };
        const rect = worldObjectRect(object);
        expect(worldObjectPosition(object, rect)).toEqual({ x: -20, y: 30 });
        const position = worldObjectPosition(object, { x: rect.x + 4, y: rect.y - 3.125 });
        expect(position).toEqual({ x: -16, y: 25 });
        expect(
          worldObjectRect({ ...object, placement: { ...object.placement, ...position } })
        ).toEqual({ ...rect, x: rect.x + 4, y: rect.y - 3.125 });
        expect(object.surface.elevation).toBe(7);
      }
  expect(worldObjectPosition(original, { x: 12, y: 24 })).toEqual({ x: 12, y: 40 });
});
