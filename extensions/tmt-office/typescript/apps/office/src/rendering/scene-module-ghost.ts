import { Container, Graphics, GraphicsPath } from 'pixi.js';
import type { ModuleSlot } from '../world-map/module-contract.js';
import { moduleSlotBounds } from '../world-map/module-geometry.js';
import { flatProjection } from './world-projection.js';
import { WORLD_WALL_RISE } from './world-geometry.js';
import { sceneLabel } from './scene-label.js';

export function moduleGhostGeometry(slot: ModuleSlot, projection = flatProjection) {
  const floor = projection.projectModuleFloor(moduleSlotBounds(slot, projection.version));
  return {
    floor,
    bounds:
      projection.version >= 6
        ? { ...floor, height: floor.height + 4 }
        : { ...floor, y: floor.y - WORLD_WALL_RISE, height: floor.height + WORLD_WALL_RISE },
  };
}

/** A disposable hologram of the real module footprint and wall rise, not saved geometry. */
export function drawModuleGhost(
  parent: Container,
  slot: ModuleSlot,
  selected: boolean,
  scale: number,
  projection = flatProjection
) {
  const { floor } = moduleGhostGeometry(slot, projection);
  const { x, y, width, height } = floor;
  const far = y - WORLD_WALL_RISE;
  const front = y + height;
  const ink = '#5ce7ee';
  const surface = new Graphics();
  parent.addChild(surface);
  if (projection.version >= 6) {
    surface.rect(x, y, width, height).fill({ color: ink, alpha: selected ? 0.14 : 0.055 });
    for (let gx = x + 4; gx < x + width; gx += 4) surface.moveTo(gx, y).lineTo(gx, front);
    for (let gy = y + 3.5; gy < front; gy += 3.5) surface.moveTo(x, gy).lineTo(x + width, gy);
    surface.stroke({ color: ink, alpha: 0.18, width: 0.65 / scale });
    surface.rect(x, y, width, height).stroke({ color: ink, alpha: 0.9, width: 1.5 / scale });
    surface
      .moveTo(x, front)
      .lineTo(x, front + 4)
      .lineTo(x + width, front + 4)
      .lineTo(x + width, front)
      .stroke({ color: ink, alpha: 0.45, width: 1 / scale });
    const cx = x + width / 2,
      cy = y + height / 2;
    surface
      .circle(cx, cy, 3)
      .moveTo(cx - 1.6, cy)
      .lineTo(cx + 1.6, cy)
      .moveTo(cx, cy - 1.6)
      .lineTo(cx, cy + 1.6)
      .stroke({ color: ink, width: 1.5 / scale });
    sceneLabel(
      parent,
      projection.version === 8
        ? 'Add area'
        : slot.type === 'meeting'
          ? 'Create meeting room'
          : 'Add office',
      cx,
      cy + 5,
      2.5,
      ink,
      width - 8
    ).anchor.set(0.5, 0);
    return floor;
  }
  surface.rect(x, y, width, height).fill({ color: ink, alpha: selected ? 0.14 : 0.055 });
  surface.poly([x, far, x + width, far, x + width, y, x, y]).fill({ color: ink, alpha: 0.08 });
  surface
    .poly([x, far, x + 3, far + 3, x + 3, front, x, front - 4])
    .fill({ color: ink, alpha: 0.1 });
  surface
    .poly([x + width, far, x + width - 3, far + 3, x + width - 3, front, x + width, front - 4])
    .fill({ color: ink, alpha: 0.1 });
  const lines = new Graphics();
  parent.addChild(lines);
  for (let gx = x + 4; gx < x + width; gx += 4) lines.moveTo(gx, y).lineTo(gx, front);
  for (let gy = y; gy <= front; gy += 2.5) lines.moveTo(x, gy).lineTo(x + width, gy);
  lines.stroke({ color: ink, alpha: 0.18, width: 0.65 / scale });
  // All room walls share a rise; corner posts expose volume.
  const frame = new GraphicsPath()
    .rect(x, far, width, 2)
    .rect(x, front - WORLD_WALL_RISE, width, WORLD_WALL_RISE);
  for (const px of [x, x + width - 3]) {
    frame.poly([
      px,
      far + 2,
      px + 1,
      far,
      px + 3,
      far,
      px + 3,
      far + 4,
      px + 2,
      far + 5,
      px,
      far + 3,
      px,
      far + 2,
    ]);
    frame
      .moveTo(px, far + 3)
      .lineTo(px, front - 2)
      .lineTo(px + 2, front)
      .lineTo(px + 3, front - 1)
      .lineTo(px + 3, far + 4);
  }
  // stroke consumes its active path; reuse the immutable frame for both passes.
  lines.path(frame).stroke({ color: ink, alpha: selected ? 0.22 : 0.12, width: 5 / scale });
  lines.path(frame).stroke({ color: ink, alpha: selected ? 1 : 0.75, width: 1.3 / scale });
  const cx = x + width / 2,
    cy = y + height / 2;
  lines
    .circle(cx, cy, 3)
    .moveTo(cx - 1.6, cy)
    .lineTo(cx + 1.6, cy)
    .moveTo(cx, cy - 1.6)
    .lineTo(cx, cy + 1.6);
  lines.stroke({ color: ink, alpha: 0.95, width: 1.5 / scale });
  sceneLabel(
    parent,
    projection.version === 8
      ? 'Add area'
      : slot.type === 'meeting'
        ? 'Create meeting room'
        : 'Add office',
    cx,
    cy + 5,
    2.5,
    ink,
    width - 8
  ).anchor.set(0.5, 0);
  return floor;
}
