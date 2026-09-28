import { Container, Graphics, Sprite, TilingSprite } from 'pixi.js';
import type { Texture } from 'pixi.js';
import type { PlatformTextures } from './platform-art.js';
import type { WallRun } from './world-geometry.js';
import type { SceneRect } from './office-geometry.js';

/** Fixed trim dimensions: a Lobby repeats the kit, never enlarges its hardware. */
export const PLATFORM_ART_SCALE = 0.045;

export function platformContour(rect: SceneRect): number[] {
  const { x, y, width: w, height: h } = rect;
  const c = Math.min(1.5, w / 4, h / 4);
  return [
    x + c,
    y,
    x + w - c,
    y,
    x + w,
    y + c,
    x + w,
    y + h - c,
    x + w - c,
    y + h,
    x + c,
    y + h,
    x,
    y + h - c,
    x,
    y + c,
  ];
}

export function drawPlatformEdge(
  parent: Container,
  wall: WallRun,
  bounds: SceneRect,
  art: PlatformTextures,
  registerLight?: (light: Graphics) => void,
  meeting = false
) {
  const group = new Container();
  group.eventMode = 'none';
  group.position.set(bounds.x, bounds.y);
  parent.addChild(group);
  const horizontal = wall.axis === 'horizontal';
  const length = horizontal ? bounds.width : bounds.height;
  if (length <= 0) return group;
  if (wall.open) {
    // Layered falloff is static geometry, not a full-scene blur or idle ticker.
    const glow = new Graphics();
    glow.eventMode = 'none';
    glow.position.set(bounds.width / 2, bounds.height / 2);
    if (!horizontal) glow.rotation = Math.PI / 2;
    for (let step = 16; step >= 1; step--) {
      const spread = step / 16;
      glow
        .ellipse(0, 0, length * (0.34 + spread * 0.48), 0.45 + spread * 5)
        .fill({ color: '#ffc354', alpha: 0.025 + (1 - spread) * 0.025 });
    }
    // A threshold lies across the actual opening, not in front of a second door.
    // Authored end posts overlap the rail ends and hide the floor/trim seam.
    const dock = new Sprite(art.bridgeDock);
    dock.anchor.set(0.5);
    dock.scale.set(length / art.bridgeDock.width);
    dock.position.set(bounds.width / 2, bounds.height / 2);
    if (!horizontal) dock.rotation = Math.PI / 2;
    group.addChild(dock);
    // Light spills onto the threshold too; placing it behind the opaque dock
    // hides its bright center and makes the falloff imperceptible at normal zoom.
    group.addChild(glow);
    registerLight?.(glow);
    return group;
  }
  if (bounds.width <= 0 || bounds.height <= 0) return group;
  function tile(texture: Texture, width: number, height: number, x = 0, y = 0) {
    const sprite = new TilingSprite({ texture, width, height });
    sprite.position.set(x, y);
    sprite.tileScale.set(PLATFORM_ART_SCALE);
    group.addChild(sprite);
    return sprite;
  }
  function detail(texture: Texture, x: number, y: number, mirrored = false) {
    const sprite = new Sprite(texture);
    sprite.scale.set(mirrored ? -PLATFORM_ART_SCALE : PLATFORM_ART_SCALE, PLATFORM_ART_SCALE);
    sprite.position.set(x, y);
    group.addChild(sprite);
  }
  if (wall.circulation) {
    // One lower outboard girder, not another rail centered under the brass.
    const support = new Graphics();
    support.eventMode = 'none';
    const center = horizontal ? 0.45 : bounds.width / 2;
    const outside = (wall.facing === 'positive' ? -1 : 1) * (horizontal ? -1 : 1);
    if (horizontal) {
      support.rotation = -Math.PI / 2;
      support.position.set(0, center + 0.6);
    } else {
      support.position.set(center, 0.6);
    }
    support.scale.x = outside;
    support.rect(0.3, 0, 1.9, length).fill('#0d2530');
    support.rect(0.45, 0, 1.35, length).fill('#284b54');
    support.rect(1.8, 0, 0.4, length).fill('#132f3a');
    for (let along = 0; along < length; along += 10.8) {
      const span = Math.min(10.8, length - along);
      support.rect(0.55, along + 0.15, 1.1, Math.max(0, span - 0.3)).fill('#385b62');
      support.rect(1.58, along + 0.3, 0.12, Math.max(0, span - 0.6)).fill('#76958a');
      if (along > 0) support.rect(0.45, along, 1.35, 0.25).fill('#102a34');
    }
    // Only the ends widen into platform attachment shoes.
    if (length >= 4)
      for (const along of [0, length - 1.5]) {
        support.rect(0.35, along, 2.25, 1.5).fill('#163640');
        support.rect(0.45, along, 1.9, 1.15).fill('#466970');
        support.rect(0.45, along, 1.9, 0.16).fill('#819f94');
      }
    group.addChild(support);
    // Bridge rails retain authored brass joints and lamp spacing in both axes.
    const rail = tile(art.bridgeRail, 0.9, length);
    if (horizontal) {
      rail.rotation = -Math.PI / 2;
      rail.y = 0.9;
    } else rail.x = center - 0.45;
    return group;
  }
  if (!horizontal) {
    // Rotate a straight armor strip, not the tapered source side silhouette:
    // repeating a taper would create a saw-tooth edge along a large Lobby.
    const armor = tile(art.face, length, 1.8);
    armor.rotation = Math.PI / 2;
    armor.x = 1.8;
    const rim = tile(art.rim, length, 0.9);
    rim.tileScale.set(0.09);
    rim.rotation = Math.PI / 2;
    rim.x = wall.facing === 'positive' ? 1.8 : 0.9;
    return group;
  }
  tile(art.rim, length, 1.1).tileScale.set(0.09);
  if (wall.raised) return group;
  tile(art.face, length, 3.3, 0, 1.1);
  // Lamps and brackets are fixed-scale accents, centered within each repeat.
  for (let x = 4; x + 7 <= length; x += 16) {
    detail(meeting ? art.meetingLamp : art.lamp, x, 0);
    if (x + 14 <= length) detail(art.bracket, x + 8, -0.2);
  }
  const corner = meeting ? art.meetingCorner : art.corner;
  if (wall.frontCorners?.start) detail(corner, 0, -0.25);
  if (wall.frontCorners?.end) detail(corner, length, -0.25, true);
  return group;
}
