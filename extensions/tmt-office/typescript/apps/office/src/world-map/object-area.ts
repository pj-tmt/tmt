import type { indexFloor } from './floor-index.js';
import { wallInteriorTile } from './map-geometry.js';
import type { WorldObject } from './world-contract.js';
import { floorObjectBounds } from './object-base.js';

/** Physical anchor only; neither resource ownership nor placement validation. */
export function objectArea(floor: ReturnType<typeof indexFloor>, object: WorldObject) {
  const tile =
    object.surface.type === 'wall'
      ? wallInteriorTile({ ...object.placement, axis: object.surface.axis }, object.surface.face)
      : floorObjectBounds(object);
  return floor.areaAt(Math.floor(tile.x), Math.floor(tile.y));
}
