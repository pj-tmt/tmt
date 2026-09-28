import { expect, it } from 'vitest';
import { platformProjection, PLATFORM_BRIDGE_LENGTH } from './platform-projection.js';
import { skybridgeCirculation } from '../world-map/compact-circulation.js';

it('projects unified grid slots independently of neighbors, purpose and source order', () => {
  const empty = platformProjection([], 7 / 8, 8);
  const full = platformProjection(
    [
      { x: 0, y: 0, width: 104, height: 88 },
      ...[-2, -1, 0, 1, 2, 3].flatMap((column) =>
        [-2, 0, 1, 4].map((row) => ({ x: column * 56, y: row * 48, width: 48, height: 40 }))
      ),
    ],
    7 / 8,
    8
  );
  for (const column of [-2, -1, 0, 1, 2, 3])
    for (const row of [-2, -1, 0, 1, 2, 4]) {
      const rect = { x: column * 56, y: row * 48, width: 48, height: 40 };
      const expected = { x: column * 72, y: row * 56, width: 48, height: 35 };
      expect(empty.projectModuleFloor(rect)).toEqual(expected);
      expect(full.projectModuleFloor(rect)).toEqual(expected);
      for (const dx of [0, 24, 48, 52, 56])
        for (const dy of [0, 20, 40, 44, 48]) {
          const point = { x: rect.x + dx, y: rect.y + dy };
          expect(full.unprojectGround(full.projectGround(point))).toEqual(point);
        }
    }
  expect(full.projectModuleFloor({ x: 0, y: 0, width: 104, height: 88 })).toEqual({
    x: 0,
    y: 0,
    width: 120,
    height: 91,
  });
});

it('keeps extended-row links narrow instead of stretching a perimeter floor band', () => {
  const rooms = [
    { x: 0, y: 0, width: 104, height: 88 },
    ...[0, 1, 2, 3].map((column) => ({ x: column * 56, y: -48, width: 48, height: 40 })),
  ];
  const projection = platformProjection(rooms, 7 / 8);
  const links = skybridgeCirculation(rooms).passages;
  expect(links).toHaveLength(5);
  for (const link of links) {
    const rect = projection.projectGroundRect(link);
    expect(rect.width <= 8 || rect.height <= 7).toBe(true);
    expect(link.width).toBe(8);
    expect(link.height).toBe(8);
  }
  expect(links.some((link) => link.x >= 104 && link.y === -8)).toBe(false);
});

it('uses one standard bridge span while retaining room dimensions and the Lobby origin', () => {
  const rooms = [
    { x: 0, y: -48, width: 48, height: 40 },
    { x: 0, y: 0, width: 104, height: 88 },
    { x: 0, y: 96, width: 48, height: 40 },
  ];
  const projection = platformProjection(rooms, 7 / 8);
  expect(PLATFORM_BRIDGE_LENGTH).toBe(24);
  expect(projection.projectGround({ x: 0, y: 0 })).toEqual({ x: 0, y: 0 });
  for (const room of rooms) {
    const rendered = projection.projectModuleFloor(room);
    expect(rendered.width).toBe(room.width);
    expect(rendered.height).toBe((room.height * 7) / 8);
  }
  for (const y of [-8, 88]) {
    const bridge = projection.projectGroundRect({ x: 20, y, width: 8, height: 8 });
    expect(bridge.height).toBe((24 * 7) / 8);
    expect(bridge.width).toBe(8);
  }
  for (const y of [-100, -48, -8, -4.5, 0, 44, 88, 92.5, 96, 200]) {
    const point = { x: 20, y };
    expect(projection.unprojectGround(projection.projectGround(point))).toEqual(point);
  }
});

it('uses the same connector length horizontally and does not stretch sprite artwork', () => {
  const projection = platformProjection(
    [
      { x: -56, y: 0, width: 48, height: 40 },
      { x: 0, y: 0, width: 104, height: 88 },
    ],
    7 / 8
  );
  expect(projection.projectGroundRect({ x: -8, y: 16, width: 8, height: 8 }).width).toBe(24);
  const art = projection.projectUpright({ x: -48, y: 8, width: 12, height: 16 });
  expect(art.width).toBe(12);
  expect(art.height).toBe(16);
  for (const x of [-64, -8, -3.5, 0, 120]) {
    const point = { x, y: 22 };
    expect(projection.unprojectGround(projection.projectGround(point))).toEqual(point);
  }
});

it('keeps island slots evenly spaced and invertible without moving other islands or campus bridges', () => {
  const campus = [
    { x: 0, y: -48, width: 48, height: 40 },
    { x: 0, y: 0, width: 104, height: 88 },
    { x: 0, y: 96, width: 48, height: 40 },
  ];
  const islands = [0, 1, 2, 3, 4].map((slot) => ({
    x: 136,
    y: slot * 48,
    width: 48,
    height: 40,
  }));
  const full = platformProjection([...campus, ...islands], 7 / 8, 7);
  const sparse = platformProjection([...campus, islands[0]!, islands[4]!], 7 / 8, 7);
  const empty = platformProjection(campus, 7 / 8, 7);
  const legacyCampus = platformProjection(campus, 7 / 8);
  for (const [index, room] of islands.entries()) {
    const expected = { x: 136, y: index * 56, width: 48, height: 35 };
    expect(full.projectModuleFloor(room)).toEqual(expected);
    expect(sparse.projectModuleFloor(room)).toEqual(expected);
    expect(empty.projectModuleFloor(room)).toEqual(expected);
  }
  for (const room of campus)
    expect(full.projectModuleFloor(room)).toEqual(legacyCampus.projectModuleFloor(room));
  for (const y of [-48, -8, 0, 40, 43.5, 48, 88, 96, 192, 232, 240]) {
    for (const x of [20, 112, 136, 160, 184, 224]) {
      const point = { x, y };
      expect(full.unprojectGround(full.projectGround(point))).toEqual(point);
      expect(sparse.projectGround(point)).toEqual(full.projectGround(point));
    }
  }
});

it('reserves rigid island widths before creation even with offices across the northern wing', () => {
  const campus = [
    { x: 0, y: 0, width: 104, height: 88 },
    ...[0, 1, 2, 3, 4].map((column) => ({ x: column * 56, y: -48, width: 48, height: 40 })),
  ];
  const room = { x: 136, y: 192, width: 48, height: 40 };
  const empty = platformProjection(campus, 7 / 8, 7);
  const occupied = platformProjection([...campus, room], 7 / 8, 7);
  expect(empty.projectModuleFloor(room)).toEqual({ x: 152, y: 224, width: 48, height: 35 });
  expect(occupied.projectModuleFloor(room)).toEqual(empty.projectModuleFloor(room));
  for (const office of campus)
    expect(occupied.projectModuleFloor(office)).toEqual(empty.projectModuleFloor(office));
  for (const x of [104, 112, 136, 160, 168, 184, 200, 224]) {
    const point = { x, y: 210 };
    expect(occupied.unprojectGround(occupied.projectGround(point))).toEqual(point);
  }
});
