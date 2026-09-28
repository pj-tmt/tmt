import { TilingSprite } from 'pixi.js';
import type { Texture } from 'pixi.js';
import type { SceneRect } from './office-geometry.js';
import { FLOOR_DEPTH } from './world-projection.js';

/** One material scale for rooms, circulation and the camera-following backdrop. */
export function floorTileScale(texture: Texture, zoom = 1): number {
  return (24 * zoom) / texture.width;
}

export function createSceneFloor(texture: Texture, bounds: SceneRect): TilingSprite {
  const surface = new TilingSprite({ texture, width: bounds.width, height: bounds.height });
  surface.position.set(bounds.x, bounds.y);
  surface.tileScale.set(floorTileScale(texture), floorTileScale(texture) * FLOOR_DEPTH);
  // Align the repeating material across room thresholds without stretching it.
  surface.tilePosition.set(-bounds.x, -bounds.y);
  return surface;
}
