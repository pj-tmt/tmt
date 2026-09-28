import { Container, Graphics, NineSliceSprite, Sprite } from 'pixi.js';
import type { Texture } from 'pixi.js';
import { ARCHITECTURE_BORDERS, ARCHITECTURE_FRAMES } from './architecture-art.js';
import { wallProjection } from './world-geometry.js';
import type { WallRun } from './world-geometry.js';
import { flatProjection } from './world-projection.js';

type WallArt = keyof typeof ARCHITECTURE_BORDERS;
export type WallTextures = Record<WallArt | 'bridgeDeck' | 'bridgeRail' | 'bridgeDock', Texture>;
const SOURCE_PIXEL_SCALE = 0.08;

/** The same derived wall owns its cutaway bounds, source art and paint depth. */
export function drawWall(
  parent: Container,
  wall: WallRun,
  textures: WallTextures,
  projection = flatProjection,
  editing = false
) {
  const { art, bounds, depth } = wallProjection(wall, projection);
  if (projection.version >= 6) {
    const sprite = new Sprite({ texture: textures.rail });
    sprite.eventMode = 'none';
    sprite.position.set(bounds.x, bounds.y);
    if (wall.open) {
      // Platform docks are painted by scene-platform, never by legacy wall art.
      sprite.width = 0;
      sprite.height = 0;
      parent.addChild(sprite);
      return { sprite, depth };
    }
    if (wall.axis === 'vertical') {
      sprite.rotation = Math.PI / 2;
      sprite.x += bounds.width;
      sprite.width = bounds.height;
      sprite.height = bounds.width;
    } else {
      sprite.width = bounds.width;
      sprite.height = bounds.height;
    }
    parent.addChild(sprite);
    if (wall.circulation && !wall.open) {
      // Flush guide lights belong to exposed bridge edges, never entrances.
      // Static pixels avoid per-frame animation and blur-filter render targets.
      const lights = new Graphics();
      lights.eventMode = 'none';
      const horizontal = wall.axis === 'horizontal';
      const length = horizontal ? bounds.width : bounds.height;
      for (let offset = 2; offset + 2 <= length; offset += 12) {
        const x = horizontal ? bounds.x + offset : bounds.x + bounds.width / 2 - 0.35;
        const y = horizontal ? bounds.y + 0.2 : bounds.y + offset;
        const width = horizontal ? 1.8 : 0.7;
        const height = horizontal ? 0.7 : 1.8;
        lights
          .rect(x - 0.7, y - 0.7, width + 1.4, height + 1.4)
          .fill({ color: '#ffb443', alpha: 0.18 });
        lights.rect(x, y, width, height).fill('#ffc35a');
        lights.rect(x + 0.15, y + 0.15, width - 0.3, height - 0.3).fill('#fff0bf');
      }
      parent.addChild(lights);
    }
    return { sprite, depth };
  }
  const side = (art === 'left' || art === 'right') && !wall.circulation;
  const paintedArt = side ? (art === 'left' ? 'leftBody' : 'rightBody') : art;
  const crownHeight = side && wall.sideStart !== false ? Math.min(5, bounds.height / 2) : 0;
  // Terminal posts own the silhouette; infill must not show through their
  // transparent outer corners as a rectangular wood-colored rim.
  const postWidth = art === 'sill' && !wall.circulation ? Math.min(6, bounds.width / 2) : 0;
  const insetLeft = wall.frontCorners?.start ? postWidth : 0;
  const insetRight = wall.frontCorners?.end ? postWidth : 0;
  const infillWidth = Math.max(0, bounds.width - insetLeft - insetRight);
  const [leftWidth, rightWidth, topHeight, bottomHeight] = ARCHITECTURE_BORDERS[paintedArt];
  // A narrow opening must retain its transparent center. Nine-slice's default
  // border compression consumes that center when the two jambs exceed its width.
  const scaleX = Math.min(SOURCE_PIXEL_SCALE, bounds.width / ARCHITECTURE_FRAMES[paintedArt].width);
  // Solid walls retain their crown/base sizes as the infill gets shorter.
  // Portals instead retain a proportional opening, including narrow jambs.
  const sourceHeight =
    art === 'back' || art === 'sill'
      ? topHeight + bottomHeight + 1
      : ARCHITECTURE_FRAMES[paintedArt].height;
  const scaleY = Math.min(SOURCE_PIXEL_SCALE, (bounds.height - crownHeight) / sourceHeight);
  const sprite = new NineSliceSprite({
    texture: textures[paintedArt],
    leftWidth,
    rightWidth,
    topHeight,
    bottomHeight,
    width: infillWidth / scaleX,
    height: (bounds.height - crownHeight) / scaleY,
  });
  sprite.scale.set(scaleX, scaleY);
  sprite.position.set(bounds.x + insetLeft, bounds.y + crownHeight);
  sprite.alpha = editing ? 0.25 : 1;
  parent.addChild(sprite);
  if (crownHeight) {
    const crown = new Sprite({ texture: textures[art === 'left' ? 'leftCrown' : 'rightCrown'] });
    crown.position.set(bounds.x, bounds.y);
    crown.width = bounds.width;
    crown.height = crownHeight;
    crown.alpha = sprite.alpha;
    parent.addChild(crown);
  }
  if (art === 'sill' && !wall.circulation && wall.frontCorners) {
    const width = Math.min(6, bounds.width / 2);
    for (const [present, x] of [
      [wall.frontCorners.start, bounds.x],
      [wall.frontCorners.end, bounds.x + bounds.width - width],
    ] as const) {
      if (!present) continue;
      const post = new Sprite({ texture: textures.frontPost });
      post.position.set(x, bounds.y);
      post.width = width;
      post.height = bounds.height;
      post.alpha = editing ? 0.5 : 1;
      parent.addChild(post);
    }
  }
  return { sprite, depth };
}
