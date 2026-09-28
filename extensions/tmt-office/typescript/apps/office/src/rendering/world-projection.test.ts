import { expect, it } from 'vitest';
import { moduleGhostGeometry } from './scene-module-ghost.js';
import {
  projectGround,
  projectGroundRect,
  projectUpright,
  unprojectGround,
  createWorldProjection,
} from './world-projection.js';

it('keeps platform ghosts flat and bridge seams on the same invertible plane', () => {
  const projection = createWorldProjection(6, { bounds: new Map(), areaAt: () => undefined });
  const ghost = moduleGhostGeometry({ type: 'meeting', index: 1 }, projection);
  expect(ghost.bounds).toEqual({ ...ghost.floor, height: ghost.floor.height + 4 });
  const room = projection.projectModuleFloor({ x: 0, y: -48, width: 48, height: 40 });
  const bridge = projection.projectGroundRect({ x: 20, y: -8, width: 8, height: 8 });
  expect(room.y + room.height).toBe(bridge.y);
  expect(bridge.y + bridge.height).toBe(0);
  const art = { x: 4, y: -40, width: 16, height: 16 };
  const painted = projection.projectUpright(art);
  expect(painted.y + painted.height).toBe(projection.projectGround({ x: 12, y: -24 }).y);
  expect(projection.uprightAnchorOffset(16)).toBe(16);
  for (const y of [-48, -8, 0, 40, 88]) {
    const point = { x: 24, y };
    expect(projection.unprojectGround(projection.projectGround(point))).toEqual(point);
  }
});

it('shortens compact floor depth while keeping upright art rigid and editing invertible', () => {
  const room = { x: 0, y: -48, width: 48, height: 40 };
  const projection = createWorldProjection(5, {
    bounds: new Map([['office', room]]),
    areaAt: (x, y) => (x >= 0 && x < 48 && y >= -48 && y < -8 ? 'office' : null),
  });
  expect(projection.projectModuleFloor(room)).toEqual({ x: 0, y: -26, width: 48, height: 19 });
  for (const point of [
    { x: 12, y: -47.5 },
    { x: 12, y: -30 },
    { x: 12, y: -8.5 },
    { x: 50, y: 44 },
  ]) {
    const restored = projection.unprojectGround(projection.projectGround(point));
    expect(restored.x).toBe(point.x);
    expect(restored.y).toBeCloseTo(point.y, 10);
  }
  const art = { x: 4, y: -40, width: 16, height: 16 };
  const painted = projection.projectUpright(art);
  expect(painted.width).toBe(16);
  expect(painted.height).toBe(16);
  expect(painted.y + painted.height).toBeCloseTo(projection.projectGround({ x: 12, y: -32 }).y, 10);
  expect(room).toEqual({ x: 0, y: -48, width: 48, height: 40 });
});

it('keeps compact meeting ghosts on the same source lattice as saved rooms', () => {
  const rooms = { bounds: new Map(), areaAt: () => undefined };
  const compact = createWorldProjection(5, rooms);
  const retained = createWorldProjection(4, rooms);
  expect(compact.version).toBe(5);
  expect(moduleGhostGeometry({ type: 'meeting', index: 1 }, compact).bounds).toEqual({
    x: 120,
    y: 35,
    width: 48,
    height: 35,
  });
  expect(moduleGhostGeometry({ type: 'meeting', index: 1 }, retained).bounds.y).toBe(48);
});

it('compresses physical depth without changing stored coordinates or upright raster proportions', () => {
  const floor = { x: 56, y: -48, width: 48, height: 40 };
  expect(projectGroundRect(floor)).toEqual({ x: 56, y: -30, width: 48, height: 25 });
  const prop = { x: 60, y: -40, width: 12, height: 16 };
  const art = projectUpright(prop);
  expect(art).toEqual({ x: 60, y: -31, width: 12, height: 16 });
  expect(art.y + art.height).toBe(projectGround({ x: 60, y: -24 }).y);
  expect(prop).toEqual({ x: 60, y: -40, width: 12, height: 16 });
});

it('keeps grid rooms rigid while exposing continuous horizontal corridors behind tall walls', () => {
  const grid = createWorldProjection(3);
  expect(grid.projectGroundRect({ x: 0, y: -48, width: 48, height: 40 })).toEqual({
    x: 0,
    y: -45,
    width: 48,
    height: 25,
  });
  expect(grid.projectGroundRect({ x: 0, y: -8, width: 104, height: 8 })).toEqual({
    x: 0,
    y: -20,
    width: 104,
    height: 20,
  });
  expect(grid.projectGroundRect({ x: 0, y: 40, width: 104, height: 8 })).toEqual({
    x: 0,
    y: 25,
    width: 104,
    height: 20,
  });
  const art = grid.projectUpright({ x: 60, y: -40, width: 12, height: 16 });
  expect(art).toEqual({ x: 60, y: -46, width: 12, height: 16 });
  for (const y of [-4096, -96, -56, -48, -8, -4.5, -0.5, 0, 39.5, 40, 44, 48, 4096]) {
    const ground = { x: -17, y };
    expect(grid.unprojectGround(grid.projectGround(ground))).toEqual(ground);
  }
});

it('round-trips signed fractional floor points and preserves contiguous seams', () => {
  for (const x of [-4096, -0.5, 0, 104, 4096])
    for (const y of [-4096, -0.5, 0, 40.5, 4096])
      expect(unprojectGround(projectGround({ x, y }))).toEqual({ x, y });
  const room = projectGroundRect({ x: 0, y: -48, width: 48, height: 40 });
  const passage = projectGroundRect({ x: 20, y: -8, width: 8, height: 8 });
  expect(room.y + room.height).toBe(passage.y);
  expect(passage.y + passage.height).toBe(0);
});
