import { catalogFurniture, footprint, validPlacement } from '../blocks/block-contract.js';
import { WALL_DIGEST, MODULAR_MOUNTED_DIGEST } from '../props/prop-contract.js';
import type { CatalogPack } from '../props/prop-contract.js';
import { projectMap, wallInteriorTile } from './map-geometry.js';
import type { MapGeometry } from './map-geometry.js';
import type { MapEdge } from './map-contract.js';
import { MAP_LIMITS } from './map-contract.js';
import { mapGeometry } from './map-source.js';
import { WORLD_LIMITS } from './world-contract.js';
import type { WorldDocument, WorldObject } from './world-contract.js';
import { floorObjectBounds } from './object-base.js';
import { withFurnitureBase } from './furniture-base.js';

/** Authoring hints only: library grouping and default kind share one owner.
 * Artwork cannot grant placement authority; native Save still validates mounts.
 */
const WALL_PRESETS: Readonly<Record<string, Readonly<Record<string, WorldObject['kind']>>>> = {
  [WALL_DIGEST]: { 'observatory-window': 'window', 'brass-wall-lamp': 'wallLight' },
  [MODULAR_MOUNTED_DIGEST]: { 'mounted-window': 'window', 'mounted-sconce': 'wallLight' },
};

/** Immediate drag feedback matching native spatial admission. Ordinary floor
 * objects may overlap (for example a desk on a rug); only window mounts conflict.
 * Native Save still owns final resource and whole-document admission.
 */
export function placementProblem(
  geometry: MapGeometry,
  object: WorldObject,
  others: readonly WorldObject[]
): string | undefined {
  const p = object.placement;
  if (
    !validPlacement(p) ||
    Math.abs(p.x) > MAP_LIMITS.coordinate ||
    Math.abs(p.y) > MAP_LIMITS.coordinate
  )
    return 'Invalid placement';
  const size = footprint(p);
  if (object.surface.type === 'floor') {
    if (object.kind !== 'decoration') return 'This object requires a wall';
    const base = floorObjectBounds(object);
    for (let y = base.y; y < base.y + base.height; y++) {
      for (let x = base.x; x < base.x + base.width; x++) {
        if (geometry.areaAt(x, y) === undefined)
          return object.surface.base
            ? 'Keep the entire base on the platform'
            : 'Keep the entire object on the platform';
        const left = geometry.boundaryAt({ x, y, axis: 'vertical' });
        const top = geometry.boundaryAt({ x, y, axis: 'horizontal' });
        const right = geometry.boundaryAt({ x: x + 1, y, axis: 'vertical' });
        const bottom = geometry.boundaryAt({ x, y: y + 1, axis: 'horizontal' });
        if ([left, top, right, bottom].some((edge) => edge?.open)) return 'Keep the entrance clear';
        if ((x > base.x && left) || (y > base.y && top)) return 'Cannot cross a room boundary';
      }
    }
    return;
  }
  const surface = object.surface;
  if (surface.elevation < 0 || surface.elevation + size.height > WORLD_LIMITS.wallHeight)
    return 'Keep the object within the wall height';
  if (!hasObjectSupport(geometry, object)) return 'This wall cannot support the object';
  const start = surface.axis === 'horizontal' ? p.x : p.y;
  const fixed = surface.axis === 'horizontal' ? p.y : p.x;
  for (const other of others) {
    if (other.id === object.id || other.surface.type !== 'wall') continue;
    if (other.kind !== 'window' && object.kind !== 'window') continue;
    if (other.surface.axis !== surface.axis || other.surface.face !== surface.face) continue;
    const otherFixed = surface.axis === 'horizontal' ? other.placement.y : other.placement.x;
    if (otherFixed !== fixed) continue;
    const otherStart = surface.axis === 'horizontal' ? other.placement.x : other.placement.y;
    const otherSize = footprint(other.placement);
    if (
      start < otherStart + otherSize.width &&
      otherStart < start + size.width &&
      surface.elevation < other.surface.elevation + otherSize.height &&
      other.surface.elevation < surface.elevation + size.height
    )
      return 'Keep the window clear';
  }
}

export function isWallCatalog(digest: string): boolean {
  return Object.hasOwn(WALL_PRESETS, digest);
}

function supportsWallSegment(
  geometry: MapGeometry,
  edge: MapEdge,
  face: 'positive' | 'negative',
  kind: WorldObject['kind']
) {
  const wall = geometry.boundaryAt(edge);
  const interior = wallInteriorTile(edge, face);
  return Boolean(
    wall &&
    !wall.open &&
    geometry.areaAt(interior.x, interior.y) !== undefined &&
    (kind !== 'window' || wall.kind === 'exterior')
  );
}

/** Spatial support for destructive-edit previews, not complete native admission. */
export function hasObjectSupport(geometry: MapGeometry, object: WorldObject): boolean {
  const size = footprint(object.placement);
  if (object.surface.type === 'floor') {
    const base = floorObjectBounds(object);
    for (let y = base.y; y < base.y + base.height; y++)
      for (let x = base.x; x < base.x + base.width; x++)
        if (geometry.areaAt(x, y) === undefined) return false;
    return true;
  }
  for (let offset = 0; offset < size.width; offset++) {
    const edge = {
      axis: object.surface.axis,
      x: object.placement.x + (object.surface.axis === 'horizontal' ? offset : 0),
      y: object.placement.y + (object.surface.axis === 'vertical' ? offset : 0),
    };
    if (!supportsWallSegment(geometry, edge, object.surface.face, object.kind)) return false;
  }
  return true;
}

/** Authoring suggestions, not Save admission. The native world remains authoritative. */
export function suggestWallPlacement(
  world: WorldDocument,
  object: WorldObject,
  areaId: string | null
): WorldObject {
  const geometry = projectMap(world.map);
  const size = footprint(object.placement);
  const elevation = Math.min(3, WORLD_LIMITS.wallHeight - size.height);
  // Index mounted silhouettes once. Suggestions leave every existing object
  // visible; manual edits can still layer ordinary art where native rules allow.
  const occupied = new Set<string>();
  for (const other of world.objects) {
    if (other.id === object.id || other.surface.type !== 'wall') continue;
    const dimensions = footprint(other.placement);
    for (let offset = 0; offset < dimensions.width; offset++) {
      const x = other.placement.x + (other.surface.axis === 'horizontal' ? offset : 0);
      const y = other.placement.y + (other.surface.axis === 'vertical' ? offset : 0);
      for (
        let height = other.surface.elevation;
        height < other.surface.elevation + dimensions.height;
        height++
      )
        occupied.add(`${other.surface.axis}:${other.surface.face}:${x}:${y}:${height}`);
    }
  }
  // Prefer a north wall facing the camera; all other axes/faces remain usable.
  const boundaries = [...geometry.boundaries].sort(
    (a, b) =>
      Number(a.edge.axis === 'vertical') - Number(b.edge.axis === 'vertical') ||
      a.edge.y - b.edge.y ||
      a.edge.x - b.edge.x
  );
  for (const { edge } of boundaries) {
    for (const face of ['positive', 'negative'] as const) {
      let fits = elevation >= 0;
      for (let offset = 0; fits && offset < size.width; offset++) {
        const x = edge.x + (edge.axis === 'horizontal' ? offset : 0);
        const y = edge.y + (edge.axis === 'vertical' ? offset : 0);
        const tile = wallInteriorTile({ x, y, axis: edge.axis }, face);
        const interior = geometry.areaAt(tile.x, tile.y);
        fits = Boolean(
          interior === areaId &&
          supportsWallSegment(geometry, { axis: edge.axis, x, y }, face, object.kind)
        );
        for (let height = elevation; fits && height < elevation + size.height; height++)
          fits = !occupied.has(`${edge.axis}:${face}:${x}:${y}:${height}`);
      }
      if (fits)
        return {
          ...object,
          surface: { type: 'wall', axis: edge.axis, face, elevation },
          placement: { ...object.placement, x: edge.x, y: edge.y },
        };
    }
  }
  throw new Error('No suitable wall in this area. Extend the area or free wall space first.');
}

export function createCatalogObject(
  world: WorldDocument,
  pack: CatalogPack,
  key: string,
  areaId: string | null,
  id: string
): WorldObject {
  const row = mapGeometry(world.map).floor.find((span) => span.areaId === areaId);
  if (!row) throw new Error('Choose an area with floor before adding an object.');
  const object: WorldObject = {
    id,
    kind:
      world.map.version >= 6 ? 'decoration' : (WALL_PRESETS[pack.digest]?.[key] ?? 'decoration'),
    surface: { type: 'floor' },
    placement: catalogFurniture(pack, key, row.start, row.y),
    extension: null,
  };
  // Pack membership only chooses an authoring preset. User art can independently
  // choose its kind and mount; no executable or geometry fields enter prop packs.
  return world.map.version < 6 && isWallCatalog(pack.digest)
    ? suggestWallPlacement(world, object, areaId)
    : withFurnitureBase(object, world.map.version);
}
