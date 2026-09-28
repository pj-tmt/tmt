import { footprint } from '../blocks/block-contract.js';
import type { FloorSpan } from '../world-map/map-contract.js';
import { projectMap } from '../world-map/map-geometry.js';
import { mapGeometry } from '../world-map/map-source.js';
import { moduleBounds, moduleConnections } from '../world-map/module-geometry.js';
import { meetingExpansionPassages, nextMeetingSlot } from '../world-map/meeting-module.js';
import type { WorldDocument, WorldObject } from '../world-map/world-contract.js';
import { floorObjectBounds } from '../world-map/object-base.js';
import type { SceneRect } from './office-geometry.js';
import { areaActorSlots } from './actor-slots.js';
import {
  createWorldProjection,
  flatProjection,
  WORLD_WALL_RISE,
  WORLD_RAIL_RISE,
} from './world-projection.js';
import type { WorldProjection } from './world-projection.js';

const CHUNK = 32;
export { WORLD_WALL_RISE } from './world-projection.js';
export interface FloorRect extends SceneRect {
  areaId: string | null;
}
export interface WallRun extends SceneRect {
  /** Derived interior owner; only used to project its wall reserve. */
  areaId?: string | null;
  axis: 'horizontal' | 'vertical';
  open: boolean;
  exterior: boolean;
  /** A north-facing room wall has interior floor to its south. */
  raised: boolean;
  /** The positive side is south for a horizontal edge and east for a vertical one. */
  facing: 'positive' | 'negative';
  /** Exterior edge of common circulation, rendered as a low guardrail. */
  circulation: boolean;
  /** Closed side returns terminate in the front face; door jambs own themselves. */
  frontCorners?: { start: boolean; end: boolean };
  /** True physical north endpoint, not a split caused by a door or edge classification. */
  sideStart?: boolean;
}

/** One cutaway projection used for visible bounds and architectural painting. */
export function wallProjection(wall: WallRun, projection = flatProjection) {
  const ground = projection.projectGroundRect(wall, wall.areaId);
  if (projection.version >= 6) {
    if (wall.open) {
      // The brass threshold has visible area even though the opening has no wall.
      // Match the authored 150x66 dock, including posts overlapping each rail end.
      const horizontal = wall.axis === 'horizontal';
      const span = (horizontal ? ground.width : ground.height) + 1.8;
      const thickness = span * (66 / 150);
      return {
        art: 'rail' as const,
        bounds: horizontal
          ? {
              x: ground.x - 0.9,
              y: ground.y + 0.35 - thickness / 2,
              width: span,
              height: thickness,
            }
          : { x: ground.x - thickness / 2, y: ground.y - 0.9, width: thickness, height: span },
        depth: ground.y + ground.height,
      };
    }
    const height =
      wall.axis === 'horizontal' ? (wall.raised ? 1.2 : wall.circulation ? 4 : 6) : ground.height;
    return {
      art: 'rail' as const,
      bounds:
        wall.axis === 'horizontal'
          ? { x: ground.x, y: ground.y, width: ground.width, height }
          : { x: ground.x - 0.9, y: ground.y, width: 1.8, height },
      depth: ground.y + ground.height,
    };
  }
  const rise = wall.circulation ? WORLD_RAIL_RISE : WORLD_WALL_RISE;
  if (wall.axis === 'horizontal') {
    const height = rise;
    return {
      art: wall.open
        ? ('portal' as const)
        : wall.circulation
          ? ('rail' as const)
          : wall.raised
            ? ('back' as const)
            : ('sill' as const),
      bounds: { x: wall.x, y: ground.y - height, width: wall.width, height },
      depth: ground.y,
    };
  }
  const thickness = wall.circulation ? 2 : projection.version >= 3 && !wall.exterior ? 4 : 6;
  const top = ground.y - rise;
  const bottom = ground.y + ground.height;
  return {
    art: wall.open
      ? ('sidePortal' as const)
      : wall.facing === 'positive'
        ? ('left' as const)
        : ('right' as const),
    bounds: {
      x:
        projection.version >= 4 && !wall.circulation
          ? wall.x - (wall.facing === 'negative' ? thickness : 0)
          : wall.x - thickness / 2,
      y: top,
      width: thickness,
      height: Math.max(0, bottom - top),
    },
    depth: ground.y + ground.height,
  };
}
interface Chunk {
  floors: FloorRect[];
  walls: WallRun[];
  objects: Set<number>;
}

export function intersects(a: SceneRect, b: SceneRect): boolean {
  return a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y;
}

/** Projected wall art stays on the derived wall; it is not a second saved location. */
export function worldObjectRect(object: WorldObject, projection = flatProjection): SceneRect {
  const { x, y } = object.placement;
  const size = footprint(object.placement);
  if (object.surface.type === 'floor') {
    if (!object.surface.base) return projection.projectUpright({ x, y, ...size });
    const base = floorObjectBounds(object);
    const anchor = projection.projectGround({
      x: base.x + base.width / 2,
      y: base.y + base.height,
    });
    return { x: anchor.x - size.width / 2, y: anchor.y - size.height, ...size };
  }
  const ground = projection.projectWallGround({ x, y }, object.surface.axis, object.surface.face);
  const elevation = object.surface.elevation;
  return object.surface.axis === 'horizontal'
    ? { x: ground.x, y: ground.y - elevation - size.height, ...size }
    : {
        x: ground.x - size.height / 2,
        y: ground.y - elevation,
        width: size.height,
        height: size.width,
      };
}

/** Invert the object's projection at a new painted origin; its wall mount stays intact. */
export function worldObjectPosition(
  object: WorldObject,
  origin: { x: number; y: number },
  projection: WorldProjection = flatProjection
) {
  const current = worldObjectRect(object, projection);
  const offset =
    object.surface.type === 'floor'
      ? projection.uprightAnchorOffset(footprint(object.placement).height)
      : 0;
  const base =
    object.surface.type === 'floor' && object.surface.base ? floorObjectBounds(object) : undefined;
  const point = base
    ? { x: base.x + base.width / 2, y: base.y + base.height }
    : {
        x: object.placement.x,
        y: object.placement.y + offset,
      };
  const anchor =
    object.surface.type === 'floor'
      ? projection.projectGround(point)
      : projection.projectWallGround(point, object.surface.axis, object.surface.face);
  const moved = {
    x: anchor.x + origin.x - current.x,
    y: anchor.y + origin.y - current.y,
  };
  const next =
    object.surface.type === 'floor'
      ? projection.unprojectGround(moved)
      : projection.unprojectWallGround(moved, object.surface.axis, object.surface.face);
  return {
    x: Math.round(next.x - (point.x - object.placement.x)),
    y: Math.round(next.y - (point.y - object.placement.y)),
  };
}

/** Build once per world draft. Camera queries visit occupied chunks, never tile-sized display objects. */
export function worldGeometry(world: WorldDocument) {
  const map = projectMap(world.map);
  const source = world.map;
  const roomBounds = new Map(
    source.version === 1
      ? []
      : source.modules.map(
          (module) => [module.area.id, moduleBounds(module, source.version)] as const
        )
  );
  const projection = createWorldProjection(world.map.version, {
    areaAt: map.areaAt,
    bounds: roomBounds,
  });
  const objectRect = (object: WorldObject) => worldObjectRect(object, projection);
  const projectedWall = (wall: WallRun) => wallProjection(wall, projection);
  const document = mapGeometry(world.map);
  const areaRows = new Map<string, FloorSpan[]>();
  for (const span of document.floor) {
    if (span.areaId === null) continue;
    const rows = areaRows.get(span.areaId) ?? [];
    rows.push(span);
    areaRows.set(span.areaId, rows);
  }
  for (const [id, rows] of areaRows) {
    const merged: FloorSpan[] = [];
    for (const row of rows.sort((a, b) => a.y - b.y || a.start - b.start)) {
      const previous = merged.at(-1);
      if (previous?.y === row.y && previous.end === row.start)
        merged[merged.length - 1] = { ...previous, end: row.end };
      else merged.push({ ...row });
    }
    areaRows.set(id, merged);
  }
  // Index once per geometry change, not once per area/actor/camera redraw.
  const anchors = new Map(
    [...areaRows].map(([id, rows]) => {
      const row = rows[Math.floor(rows.length / 2)]!;
      return [id, { x: (row.start + row.end) / 2, y: row.y + 0.5 }] as const;
    })
  );
  const chunks = new Map<string, Chunk>();
  const projectedFloor = { left: Infinity, top: Infinity, right: -Infinity, bottom: -Infinity };
  function chunk(x: number, y: number) {
    const key = `${x},${y}`;
    let value = chunks.get(key);
    if (!value) {
      value = { floors: [], walls: [], objects: new Set() };
      chunks.set(key, value);
    }
    return value;
  }
  for (const span of document.floor) {
    const rect = projection.projectGroundRect(
      { x: span.start, y: span.y, width: span.end - span.start, height: 1 },
      span.areaId
    );
    projectedFloor.left = Math.min(projectedFloor.left, rect.x);
    projectedFloor.top = Math.min(projectedFloor.top, rect.y);
    projectedFloor.right = Math.max(projectedFloor.right, rect.x + rect.width);
    projectedFloor.bottom = Math.max(projectedFloor.bottom, rect.y + rect.height);
    // Index display chunks after projection, including expanded bridge bands.
    // Split once at chunk edges so floor coverage never duplicates or disappears.
    for (let cx = Math.floor(rect.x / CHUNK); cx * CHUNK < rect.x + rect.width; cx++) {
      for (let cy = Math.floor(rect.y / CHUNK); cy * CHUNK < rect.y + rect.height; cy++) {
        const x = Math.max(rect.x, cx * CHUNK),
          y = Math.max(rect.y, cy * CHUNK);
        chunk(cx, cy).floors.push({
          x,
          y,
          width: Math.min(rect.x + rect.width, (cx + 1) * CHUNK) - x,
          height: Math.min(rect.y + rect.height, (cy + 1) * CHUNK) - y,
          areaId: span.areaId,
        });
      }
    }
  }
  // Merge equal projected runs; floor units never become individual display objects.
  for (const part of chunks.values()) {
    const merged = new Map<string, FloorRect>();
    const result: FloorRect[] = [];
    for (const row of part.floors.sort((a, b) => a.y - b.y || a.x - b.x)) {
      const key = `${row.x}:${row.width}:${row.areaId}`;
      const previous = merged.get(key);
      if (previous && previous.y + previous.height === row.y) previous.height += row.height;
      else {
        result.push(row);
        merged.set(key, row);
      }
    }
    part.floors = result;
  }
  const edges = [...map.boundaries].sort((a, b) => {
    if (a.edge.axis !== b.edge.axis) return a.edge.axis === 'horizontal' ? -1 : 1;
    return a.edge.axis === 'horizontal'
      ? a.edge.y - b.edge.y || a.edge.x - b.edge.x
      : a.edge.x - b.edge.x || a.edge.y - b.edge.y;
  });
  let previous: WallRun | undefined;
  const walls: WallRun[] = [];
  for (const boundary of edges) {
    const { x, y, axis } = boundary.edge;
    const exterior = boundary.kind === 'exterior';
    const positive = map.areaAt(x, y);
    const negative = map.areaAt(x - Number(axis === 'vertical'), y - Number(axis === 'horizontal'));
    const areaId =
      typeof positive === 'string' ? positive : typeof negative === 'string' ? negative : null;
    const grid = world.map.version >= 3;
    const high = axis === 'horizontal' && positive !== undefined && (!grid || positive !== null);
    // Common floor is not the interior of the neighboring room. Choose the
    // room-facing side frame instead of mirroring both walls into the corridor.
    const facing =
      positive !== undefined && (!grid || positive !== null || negative === undefined)
        ? 'positive'
        : 'negative';
    const circulation = exterior && (positive === null || negative === null);
    if (
      previous &&
      previous.axis === axis &&
      previous.areaId === areaId &&
      previous.open === boundary.open &&
      previous.exterior === exterior &&
      previous.raised === high &&
      previous.facing === facing &&
      previous.circulation === circulation &&
      (axis === 'horizontal'
        ? previous.y === y && previous.x + previous.width === x
        : previous.x === x && previous.y + previous.height === y)
    ) {
      if (axis === 'horizontal') previous.width++;
      else previous.height++;
    } else {
      previous = {
        areaId,
        x,
        y,
        width: 1,
        height: 1,
        axis,
        exterior,
        open: boundary.open,
        raised: high,
        facing,
        circulation,
      };
      walls.push(previous);
    }
  }
  // Corner ownership comes from actual side-wall endpoints, not rectangular
  // room decorations. A portal split therefore cannot add a second jamb.
  const cornerKey = (owner: string | null | undefined, x: number, y: number) =>
    `${owner ?? ''}:${x}:${y}`;
  const sideEnds = new Set(
    walls
      .filter((wall) => wall.axis === 'vertical' && !wall.open && !wall.circulation)
      .map((wall) => cornerKey(wall.areaId, wall.x, wall.y + wall.height))
  );
  const sideContinuations = new Set(
    walls
      .filter((wall) => wall.axis === 'vertical')
      .map((wall) => cornerKey(wall.areaId, wall.x, wall.y + wall.height))
  );
  for (const wall of walls) {
    if (wall.axis === 'vertical')
      wall.sideStart = !sideContinuations.has(cornerKey(wall.areaId, wall.x, wall.y));
  }
  for (const wall of walls) {
    if (wall.axis !== 'horizontal' || wall.raised || wall.open || wall.circulation) continue;
    wall.frontCorners = {
      start: sideEnds.has(cornerKey(wall.areaId, wall.x, wall.y)),
      end: sideEnds.has(cornerKey(wall.areaId, wall.x + wall.width, wall.y)),
    };
  }
  function visit(rect: SceneRect, add: (part: Chunk) => void) {
    for (let y = Math.floor(rect.y / CHUNK); y <= Math.floor((rect.y + rect.height) / CHUNK); y++)
      for (let x = Math.floor(rect.x / CHUNK); x <= Math.floor((rect.x + rect.width) / CHUNK); x++)
        add(chunk(x, y));
  }
  // A module passage has two logical thresholds, but one visible portal assembly.
  // Keep the east/south frame; the opposite threshold stays open ground rather
  // than receiving a second complete arch. Physical admission still sees both.
  const thresholdKey = (edge: { x: number; y: number; axis: string }) =>
    `${edge.x},${edge.y},${edge.axis}`;
  const unframed = new Set(
    world.map.version === 2
      ? moduleConnections(world.map.modules).map(({ openings }) => thresholdKey(openings[0]))
      : []
  );
  for (const wall of walls) {
    if (wall.open && unframed.has(thresholdKey(wall))) continue;
    const bounds = projectedWall(wall).bounds;
    if (bounds.height > 0 && bounds.width > 0) visit(bounds, (part) => part.walls.push(wall));
  }
  world.objects.forEach((object, index) =>
    visit(objectRect(object), (part) => part.objects.add(index))
  );
  // Independent lanes have different Y transforms. Projecting opposite corners
  // of the whole source rectangle can clip the farther lane out of Fit.
  const raw =
    source.version >= 7
      ? {
          x: projectedFloor.left,
          y: projectedFloor.top,
          width: projectedFloor.right - projectedFloor.left,
          height: projectedFloor.bottom - projectedFloor.top,
        }
      : projection.projectGroundRect(map.bounds ?? { x: 0, y: 0, width: 36, height: 36 }, null);
  const extents = world.objects.map(objectRect);
  const left = Math.min(raw.x, ...extents.map((rect) => rect.x));
  const top = Math.min(
    raw.y - (projection.version >= 6 ? 0 : WORLD_WALL_RISE),
    ...extents.map((rect) => rect.y)
  );
  const right = Math.max(raw.x + raw.width, ...extents.map((rect) => rect.x + rect.width));
  const bottom = Math.max(raw.y + raw.height, ...extents.map((rect) => rect.y + rect.height));
  const bounds = { x: left - 4, y: top - 4, width: right - left + 8, height: bottom - top + 12 };
  const actorSlots = new Map<string, SceneRect[]>();
  const wallRows = new Map<number, WallRun[]>(),
    wallColumns = new Map<number, WallRun[]>();
  for (const wall of walls) {
    if (wall.open) continue;
    const lines = wall.axis === 'horizontal' ? wallRows : wallColumns;
    const key = wall.axis === 'horizontal' ? wall.y : wall.x;
    const line = lines.get(key) ?? [];
    line.push(wall);
    lines.set(key, line);
  }
  const objectDepths = new Map(
    world.objects.map((object) => {
      if (object.surface.type === 'floor') {
        // Stored object order is the furniture stack (rug, desk, equipment).
        // Keep it in front of its reserved rear face, but behind foreground
        // walls. Sorting individual footprints would paint rugs over desks.
        const base = floorObjectBounds(object);
        const owner = map.areaAt(base.x, base.y);
        const room =
          source.version >= 4 && typeof owner === 'string' ? roomBounds.get(owner) : undefined;
        return [
          object.id,
          room ? projection.projectModuleFloor(room).y + 0.005 : Number.NEGATIVE_INFINITY,
        ] as const;
      }
      const horizontal = object.surface.axis === 'horizontal';
      const line = (horizontal ? wallRows : wallColumns).get(
        horizontal ? object.placement.y : object.placement.x
      );
      const position = horizontal ? object.placement.x : object.placement.y;
      const wall = line?.find((item) =>
        horizontal
          ? position >= item.x && position < item.x + item.width
          : position >= item.y && position < item.y + item.height
      );
      return [
        object.id,
        (wall ? projectedWall(wall).depth : projection.projectGround(object.placement).y) + 0.01,
      ] as const;
    })
  );
  const furniture = world.objects
    .filter((object) => object.surface.type === 'floor')
    .map((object) => ({ ...object.placement, ...footprint(object.placement) }));
  const meetingSlot =
    source.version !== 1 && source.version >= 4 ? nextMeetingSlot(source) : undefined;
  const meetingPreview =
    meetingSlot && source.version !== 1
      ? { slot: meetingSlot, passages: meetingExpansionPassages(source, meetingSlot) }
      : undefined;
  return {
    meetingPreview,
    /** Floor paint stops inside the wall silhouette. Transparent atlas corners
     * must reveal the sky, not an oversized rectangular wood foundation.
     * This changes pixels only, never occupancy or placement coordinates.
     */
    floorPaintBounds(rect: FloorRect): FloorRect | undefined {
      const room = rect.areaId && roomBounds.get(rect.areaId);
      if (!room) return rect;
      const shell = projection.projectGroundRect(room, null);
      const sideInset = projection.version >= 6 ? 0 : 6;
      const depthInset = projection.version >= 6 ? 0 : 1.5;
      const x = Math.max(rect.x, shell.x + sideInset);
      const y = Math.max(rect.y, shell.y + depthInset);
      const right = Math.min(rect.x + rect.width, shell.x + shell.width - sideInset);
      const bottom = Math.min(rect.y + rect.height, shell.y + shell.height - depthInset);
      return right > x && bottom > y
        ? { x, y, width: right - x, height: bottom - y, areaId: rect.areaId }
        : undefined;
    },
    projection,
    objectRect,
    objectPosition: (object: WorldObject, origin: { x: number; y: number }) =>
      worldObjectPosition(object, origin, projection),
    map,
    bounds,
    anchors,
    /** Room plaques follow the same north wall reserve, not a floating floor label. */
    nameplate(areaId: string) {
      const room = roomBounds.get(areaId);
      if (room) {
        const ground = projection.projectGround({ x: room.x + room.width / 2, y: room.y }, areaId);
        return { x: ground.x, y: ground.y + (projection.version >= 6 ? 3 : -WORLD_WALL_RISE + 6) };
      }
      const center = anchors.get(areaId);
      if (!center) return undefined;
      const point = projection.projectGround(center);
      return { x: point.x, y: point.y + 11 };
    },
    objectDepth(object: WorldObject) {
      return objectDepths.get(object.id) ?? projection.projectGround(object.placement).y;
    },
    /** Lazy per occupied area, retained across camera redraws. */
    actorSlots(areaId: string) {
      let slots = actorSlots.get(areaId);
      if (!slots) {
        const anchor = anchors.get(areaId);
        slots = anchor
          ? areaActorSlots(areaRows.get(areaId)!, anchor, furniture, (slot) => {
              const actor = projection.projectUpright(slot);
              const depth = actor.y + actor.height;
              // Query the same spatial index and projection used for drawing.
              const occluders = new Set<WallRun>();
              visit(actor, (part) => part.walls.forEach((wall) => occluders.add(wall)));
              return ![...occluders].some((wall) => {
                // Prefer interior space to standing behind a portal's jambs or
                // lintel. Its bounding box is conservative; safe fallback remains.
                const painted = projectedWall(wall);
                return painted.depth >= depth && intersects(painted.bounds, actor);
              });
            })
          : [];
        actorSlots.set(areaId, slots);
      }
      return slots;
    },
    /** A margin covers raised walls, labels and props near the viewport edge. */
    visible(viewport: SceneRect) {
      const margin = WORLD_WALL_RISE + 8;
      const view = {
        x: viewport.x - margin,
        y: viewport.y - margin,
        width: viewport.width + margin * 2,
        height: viewport.height + margin * 2,
      };
      const floors: FloorRect[] = [],
        selectedWalls = new Set<WallRun>(),
        objects = new Set<number>();
      // Sparse iteration is bounded by admitted occupied chunks even when zoomed far out.
      for (const [key, part] of chunks) {
        const [x, y] = key.split(',').map(Number) as [number, number];
        if (!intersects(view, { x: x * CHUNK, y: y * CHUNK, width: CHUNK, height: CHUNK }))
          continue;
        floors.push(...part.floors);
        for (const wall of part.walls) selectedWalls.add(wall);
        for (const index of part.objects) objects.add(index);
      }
      return { floors, walls: [...selectedWalls], objects: [...objects].sort((a, b) => a - b) };
    },
  };
}
