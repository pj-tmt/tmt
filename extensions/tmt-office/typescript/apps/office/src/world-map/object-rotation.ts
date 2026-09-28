import { footprint } from '../blocks/block-contract.js';
import type { CatalogPack, PropDefinition } from '../props/prop-contract.js';
import { resolvePlacedProp } from '../props/prop-contract.js';
import { directionalFurniture } from '../props/furniture-upgrades.js';
import type { WorldObject } from './world-contract.js';
import { withFurnitureBase } from './furniture-base.js';
import { floorObjectBounds } from './object-base.js';

/** Repeated directional frames are static artwork, not four authored views. */
export function hasRotationArt(prop: PropDefinition): boolean {
  return !prop.frames || new Set(prop.frames.map((frame) => JSON.stringify(frame))).size > 1;
}

export function canRotateObject(object: WorldObject, catalog: readonly CatalogPack[]): boolean {
  const placement =
    object.surface.type === 'floor'
      ? directionalFurniture(object.placement, catalog)
      : object.placement;
  const resolved = resolvePlacedProp(catalog, placement);
  return Boolean(resolved && hasRotationArt(resolved.definition));
}

/** Keep the ground footprint centered, snapping its origin to the world grid. */
export function rotatedObject(
  object: WorldObject,
  turns: number,
  catalog?: readonly CatalogPack[],
  mapVersion = 1
): WorldObject {
  const before = footprint(object.placement);
  const rotation = (((object.placement.rotation + turns) % 4) + 4) % 4;
  if (rotation === object.placement.rotation) return object;
  const source =
    catalog && object.surface.type === 'floor'
      ? directionalFurniture(object.placement, catalog)
      : object.placement;
  const placement = { ...source, rotation };
  const supported = withFurnitureBase({ ...object, placement: source }, mapVersion);
  if (supported.surface.type === 'floor' && supported.surface.base) {
    const beforeBase = floorObjectBounds(supported);
    const candidate = { ...supported, placement: { ...supported.placement, rotation } };
    const afterBase = floorObjectBounds(candidate);
    return {
      ...candidate,
      placement: {
        ...candidate.placement,
        x:
          candidate.placement.x +
          Math.floor(beforeBase.x + beforeBase.width / 2) -
          Math.floor(afterBase.x + afterBase.width / 2),
        y:
          candidate.placement.y +
          Math.floor(beforeBase.y + beforeBase.height / 2) -
          Math.floor(afterBase.y + afterBase.height / 2),
      },
    };
  }
  const after = footprint(placement);
  return {
    ...object,
    placement: {
      ...placement,
      x:
        placement.x +
        Math.floor((placement.footprint.width - after.width) / 2) -
        Math.floor((placement.footprint.width - before.width) / 2),
      y:
        placement.y +
        Math.floor((placement.footprint.height - after.height) / 2) -
        Math.floor((placement.footprint.height - before.height) / 2),
    },
  };
}
