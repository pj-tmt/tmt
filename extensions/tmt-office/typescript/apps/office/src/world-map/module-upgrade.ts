import { footprint } from '../blocks/block-contract.js';
import { wallInteriorTile } from './map-geometry.js';
import { moduleBounds } from './module-geometry.js';
import { decodeWorldDocument } from './world-contract.js';
import type { WorldDocument } from './world-contract.js';
import { planFreeformModules } from './freeform-upgrade.js';
import { indexFloor } from './floor-index.js';
import { mapGeometry } from './map-source.js';
import type { ModuleMapDocument } from './module-contract.js';
import type { ModuleRect } from './module-geometry.js';
import { floorObjectBounds } from './object-base.js';

/** Explicit draft conversion. Resource contents and stored snapshots are untouched. */
export function upgradeModuleWorld(world: WorldDocument): WorldDocument {
  const source = world.map;
  if (source.version >= 4) return world;
  const modules =
    source.version === 1
      ? []
      : source.modules.map((module) => ({
          ...module,
          slot:
            module.slot.type === 'office' && module.slot.row >= 1
              ? { ...module.slot, row: module.slot.row + 1 }
              : module.slot,
        }));
  const plan =
    source.version === 1
      ? planFreeformModules(source)
      : {
          map: { ...source, version: 4 as const, modules },
          moves: source.modules.map((module, index) => ({
            areaId: module.area.id,
            before: moduleBounds(module, source.version),
            after: moduleBounds(modules[index]!, 4),
          })),
        };
  return relocateModuleObjects(world, plan);
}

/** Explicit compact-layout preview; no stored v4 coordinates are reinterpreted. */
export function compactModuleWorld(world: WorldDocument): WorldDocument {
  if (world.map.version >= 5) return world;
  const prepared = upgradeModuleWorld(world);
  const source = prepared.map;
  if (source.version === 1) throw new Error('A modular layout is required.');
  const map: ModuleMapDocument = { ...source, version: 5 };
  return relocateModuleObjects(prepared, {
    map,
    moves: source.modules.map((module) => ({
      areaId: module.area.id,
      before: moduleBounds(module, source.version),
      after: moduleBounds(module, 5),
    })),
  });
}

/** A reversible preview, never an implicit reinterpretation of a saved layout. */
export function skybridgeModuleWorld(world: WorldDocument): WorldDocument {
  if (world.map.version >= 6) return world;
  const prepared = upgradeModuleWorld(world);
  const source = prepared.map;
  if (source.version === 1) throw new Error('A modular layout is required.');
  const map: ModuleMapDocument = { ...source, version: 6 };
  return relocateModuleObjects(prepared, {
    map,
    moves: source.modules.map((module) => ({
      areaId: module.area.id,
      before: moduleBounds(module, source.version),
      after: moduleBounds(module, 6),
    })),
  });
}

/** Explicit platform draft: keep every object and resource, remove wall support. */
export function platformModuleWorld(world: WorldDocument): WorldDocument {
  const prepared = skybridgeModuleWorld(world);
  if (!prepared.objects.some((object) => object.surface.type === 'wall')) return prepared;
  const source = prepared.map;
  if (source.version === 1) throw new Error('A modular layout is required.');
  const floor = indexFloor(mapGeometry(source).floor);
  const objects = prepared.objects.map((object) => {
    if (object.surface.type === 'floor') return object;
    const tile = wallInteriorTile(
      { ...object.placement, axis: object.surface.axis },
      object.surface.face
    );
    const owner = floor.areaAt(tile.x, tile.y);
    const module = source.modules.find((value) => value.area.id === owner);
    if (!module) throw new Error(`Object ${object.id} has no owning platform.`);
    const bounds = moduleBounds(module, source.version);
    const size = footprint(object.placement);
    if (size.width > bounds.width || size.height > bounds.height)
      throw new Error(`Object ${object.id} does not fit its platform.`);
    const x = Math.max(bounds.x, Math.min(tile.x, bounds.x + bounds.width - size.width));
    const y = Math.max(bounds.y, Math.min(tile.y, bounds.y + bounds.height - size.height));
    return {
      ...object,
      kind: 'decoration' as const,
      surface: { type: 'floor' as const },
      placement: { ...object.placement, x, y },
    };
  });
  return decodeWorldDocument({ ...prepared, objects });
}

/** One explicit alignment of retained wing slots; later use changes only edit
 * the binding. Canonical rooms, resources and object-relative bases are retained. */
export function unifiedAreaWorld(world: WorldDocument): WorldDocument {
  if (world.map.version === 8) return world;
  const prepared = platformModuleWorld(world);
  const source = prepared.map;
  if (source.version === 1) throw new Error('A modular layout is required.');
  const modules = source.modules.map((module) => ({
    ...module,
    slot:
      module.slot.type === 'meeting'
        ? { type: 'office' as const, column: 2, row: module.slot.index }
        : module.slot,
  }));
  const map: ModuleMapDocument = { ...source, version: 8, modules };
  return relocateModuleObjects(prepared, {
    map,
    moves: source.modules.map((module, index) => ({
      areaId: module.area.id,
      before: moduleBounds(module, source.version),
      after: moduleBounds(modules[index]!, 8),
    })),
  });
}

function relocateModuleObjects(
  world: WorldDocument,
  plan: {
    map: ModuleMapDocument;
    moves: readonly { areaId: string; before: ModuleRect; after: ModuleRect }[];
  }
): WorldDocument {
  const originalFloor = indexFloor(mapGeometry(world.map).floor);
  const objects = world.objects.map((object) => {
    const { placement, surface } = object;
    const size = footprint(placement);
    const support =
      surface.type === 'floor'
        ? floorObjectBounds(object)
        : {
            ...wallInteriorTile({ ...placement, axis: surface.axis }, surface.face),
            width: surface.axis === 'horizontal' ? size.width : 1,
            height: surface.axis === 'vertical' ? size.width : 1,
          };
    const owner = originalFloor.areaAt(support.x, support.y);
    const move = plan.moves.find(
      ({ before, areaId }) =>
        areaId === owner &&
        support.x >= before.x &&
        support.y >= before.y &&
        support.x + support.width <= before.x + before.width &&
        support.y + support.height <= before.y + before.height
    );
    let contained = Boolean(move && owner === move.areaId);
    for (let y = support.y; contained && y < support.y + support.height; y++)
      for (let x = support.x; x < support.x + support.width; x++)
        if (originalFloor.areaAt(x, y) !== owner) contained = false;
    if (!move || !contained)
      throw new Error(
        `Move object ${object.id} fully inside a room or onto its interior wall before upgrading.`
      );
    const { before, after } = move;
    // The Lobby's south mount follows its expanded boundary, not its old floor.
    const southMount =
      surface.type === 'wall' &&
      surface.axis === 'horizontal' &&
      placement.y === before.y + before.height;
    const relativeX = support.x - before.x;
    const relativeY = support.y - before.y + (southMount ? after.height - before.height : 0);
    if (relativeX + support.width > after.width || relativeY + support.height > after.height)
      throw new Error(
        `Object ${object.id} does not fit its destination room. Reposition it before upgrading.`
      );
    return {
      ...object,
      placement: {
        ...placement,
        x: placement.x + after.x - before.x,
        y: southMount ? after.y + after.height : placement.y + after.y - before.y,
      },
    };
  });
  // Existing admission owns bounds, reserved slots and document limits. Native
  // Save still checks reachability, collisions, wall support and resource grants.
  return decodeWorldDocument({ ...world, map: plan.map, objects });
}
