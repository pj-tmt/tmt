import type { SceneRect } from './office-geometry.js';
import { MODULE_METRICS } from '../world-map/module-geometry.js';
import { wallInteriorTile } from '../world-map/map-geometry.js';
import { platformProjection } from './platform-projection.js';

/** Orthographic cutaway: floor depth contracts; upright artwork never stretches.
 * This affects pixels only. Native coordinates, occupancy and storage stay intact.
 * A binary-exact ratio also keeps adjacent floor-run seams numerically identical.
 */
export const FLOOR_DEPTH = 5 / 8;
export const WORLD_WALL_RISE = 16;
export const WORLD_RAIL_RISE = 4;

export interface ProjectionRooms {
  bounds: ReadonlyMap<string, SceneRect>;
  areaAt: (x: number, y: number) => string | null | undefined;
}

/** The cell owns its wall reserve. Public floor stays on the unexpanded lattice.
 * Bounds and ownership come from the admitted map, never a second room layout.
 */
function centralProjection(rooms: ProjectionRooms, version: number) {
  const depth = version >= 5 ? 7 / 8 : 1;
  const uprightAnchorOffset = (height: number) => (version === 5 ? height / 2 : height);
  const roomAt = (point: { x: number; y: number }) =>
    rooms.areaAt(Math.floor(point.x), Math.floor(point.y));
  const roomBounds = (owner: string | null | undefined) =>
    typeof owner === 'string' ? rooms.bounds.get(owner) : undefined;
  const floorY = (y: number, room?: SceneRect) =>
    room && version < 6
      ? room.y * depth +
        WORLD_WALL_RISE +
        ((y - room.y) * (room.height * depth - WORLD_WALL_RISE)) / room.height
      : y * depth;
  function projectGround(point: { x: number; y: number }, owner = roomAt(point)) {
    return { x: point.x, y: floorY(point.y, roomBounds(owner)) };
  }
  function unprojectGround(
    point: { x: number; y: number },
    owner = roomAt({ x: point.x, y: point.y / depth })
  ) {
    const room = roomBounds(owner);
    return {
      x: point.x,
      y:
        room && version < 6
          ? room.y +
            (Math.max(0, point.y - room.y * depth - WORLD_WALL_RISE) * room.height) /
              (room.height * depth - WORLD_WALL_RISE)
          : point.y / depth,
    };
  }
  function projectGroundRect(
    rect: SceneRect,
    owner = roomAt({
      x: rect.x + rect.width / 2,
      y: rect.y + rect.height / 2,
    })
  ): SceneRect {
    const room = roomBounds(owner);
    const y = floorY(rect.y, room);
    return { ...rect, y, height: floorY(rect.y + rect.height, room) - y };
  }
  function projectUpright(rect: SceneRect): SceneRect {
    const owner = roomAt({ x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 });
    return {
      ...rect,
      y: floorY(rect.y + uprightAnchorOffset(rect.height), roomBounds(owner)) - rect.height,
    };
  }
  function wallOwner(
    point: { x: number; y: number },
    axis: 'horizontal' | 'vertical',
    face: 'positive' | 'negative'
  ) {
    // An empty outside face is explicitly public space, not an omitted owner
    // that should fall back to the room on the opposite side of the wall.
    return roomAt(wallInteriorTile({ ...point, axis }, face)) ?? null;
  }
  return {
    version,
    uprightAnchorOffset,
    projectGround,
    unprojectGround,
    projectGroundRect,
    projectUpright,
    projectWallGround: (
      point: { x: number; y: number },
      axis: 'horizontal' | 'vertical',
      face: 'positive' | 'negative'
    ) => projectGround(point, wallOwner(point, axis, face)),
    unprojectWallGround: (
      point: { x: number; y: number },
      axis: 'horizontal' | 'vertical',
      face: 'positive' | 'negative'
    ) => unprojectGround(point, wallOwner({ x: point.x, y: point.y / depth }, axis, face)),
    projectModuleFloor: (rect: SceneRect) => ({
      ...rect,
      y: rect.y * depth + (version >= 6 ? 0 : WORLD_WALL_RISE),
      height: rect.height * depth - (version >= 6 ? 0 : WORLD_WALL_RISE),
    }),
  };
}

/** Grid corridors need visible floor behind tall cutaway walls. Only the gap
 * between rows expands in pixels; rooms, saved coordinates and art stay intact.
 * The invertible transform is shared by painting, culling, picking and dragging.
 */
export function createWorldProjection(version: number, rooms?: ProjectionRooms) {
  if (version >= 4) {
    if (!rooms) throw new Error('Central-grid projection requires admitted module ownership.');
    if (version >= 6) return platformProjection(rooms.bounds.values(), 7 / 8, version);
    return centralProjection(rooms, version);
  }
  const { roomHeight, rowStep, passageWidth } = MODULE_METRICS;
  const roomDepth = roomHeight * FLOOR_DEPTH;
  const grid = version === 3;
  const corridorDepth = grid ? 20 : passageWidth * FLOOR_DEPTH;
  const sceneStep = roomDepth + corridorDepth;
  function projectY(y: number) {
    if (!grid) return y * FLOOR_DEPTH;
    const row = Math.floor(y / rowStep),
      local = y - row * rowStep;
    return (
      row * sceneStep +
      Math.min(local, roomHeight) * FLOOR_DEPTH +
      (Math.max(0, local - roomHeight) * corridorDepth) / passageWidth
    );
  }
  function unprojectY(y: number) {
    if (!grid) return y / FLOOR_DEPTH;
    const row = Math.floor(y / sceneStep),
      local = y - row * sceneStep;
    return (
      row * rowStep +
      Math.min(local, roomDepth) / FLOOR_DEPTH +
      (Math.max(0, local - roomDepth) * passageWidth) / corridorDepth
    );
  }
  function projectGround(point: { x: number; y: number }) {
    return { x: point.x, y: projectY(point.y) };
  }
  function unprojectGround(point: { x: number; y: number }) {
    return { x: point.x, y: unprojectY(point.y) };
  }
  function projectGroundRect(rect: SceneRect, _owner?: string | null): SceneRect {
    return {
      ...rect,
      y: projectY(rect.y),
      height: projectY(rect.y + rect.height) - projectY(rect.y),
    };
  }
  /** Upright artwork remains rigid and bottom-anchored, including across a gap. */
  function projectUpright(rect: SceneRect): SceneRect {
    return { ...rect, y: projectY(rect.y + rect.height) - rect.height };
  }
  return {
    version,
    uprightAnchorOffset: (height: number) => height,
    projectGround,
    unprojectGround,
    projectGroundRect,
    projectUpright,
    projectWallGround: projectGround,
    unprojectWallGround: unprojectGround,
    projectModuleFloor: projectGroundRect,
  };
}

export type WorldProjection = ReturnType<typeof createWorldProjection>;
export const flatProjection = createWorldProjection(1);
export const { projectGround, unprojectGround, projectGroundRect, projectUpright } = flatProjection;
