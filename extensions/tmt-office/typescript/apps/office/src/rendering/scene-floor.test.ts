import { Texture } from 'pixi.js';
import { expect, it } from 'vitest';
import { createSceneFloor, floorTileScale } from './scene-floor.js';

it('tiles one shared material at a fixed world scale rather than stretching it to each room', () => {
  const texture = Texture.EMPTY;
  const hall = createSceneFloor(texture, { x: 0, y: 0, width: 108, height: 56 });
  const room = createSceneFloor(texture, { x: 38, y: 12, width: 32, height: 32 });
  try {
    expect(room.texture).toBe(hall.texture);
    expect([room.width, room.height]).toEqual([32, 32]);
    expect([hall.width, hall.height]).toEqual([108, 56]);
    expect(room.tileScale.x * texture.width).toBe(24);
    expect(room.tileScale.y * texture.width).toBe(15);
    expect(room.tileScale.x).toBe(hall.tileScale.x);
    expect(room.tileScale.y).toBe(hall.tileScale.y);
    expect([room.x + room.tilePosition.x, room.y + room.tilePosition.y]).toEqual([0, 0]);
    expect(floorTileScale(texture, 2.5)).toBe(hall.tileScale.x * 2.5);
  } finally {
    room.destroy();
    hall.destroy();
  }
});
