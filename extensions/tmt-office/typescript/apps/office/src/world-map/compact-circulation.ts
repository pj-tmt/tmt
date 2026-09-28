import { MODULE_METRICS } from './module-metrics.js';
import type { ModuleRect } from './module-geometry.js';

/** Required branches on the public lattice, not a filled map bounding box.
 * Each occupied office reaches the Lobby through a horizontal branch and the
 * central vertical spine. Private room floor is never used as a shortcut.
 * These candidate rectangles are clipped/merged by the map projection owner.
 */
export function compactCirculation(rooms: readonly ModuleRect[]): ModuleRect[] {
  const { roomWidth, roomHeight, passageWidth, lobbyWidth } = MODULE_METRICS;
  const centerX = roomWidth;
  const centerY = roomHeight;
  const lobbyHeight = 2 * roomHeight + passageWidth;
  const result: ModuleRect[] = [];
  for (const room of rooms) {
    if (room.x === 0 && room.y === 0 && room.width === lobbyWidth && room.height === lobbyHeight)
      continue;
    const branchY =
      room.y < 0 ? room.y + room.height : room.y >= lobbyHeight ? room.y - passageWidth : centerY;
    const entranceX = room.x + (room.width - passageWidth) / 2;
    result.push({
      x: Math.min(entranceX, centerX),
      y: branchY,
      width: Math.max(entranceX, centerX) + passageWidth - Math.min(entranceX, centerX),
      height: passageWidth,
    });
    result.push({
      x: centerX,
      y: Math.min(branchY, centerY),
      width: passageWidth,
      height: Math.max(branchY, centerY) + passageWidth - Math.min(branchY, centerY),
    });
  }
  return result;
}

/** V6 connects immediate cardinal neighbors only. Platforms can be traversed;
 * there is no perimeter bypass or bridge across an empty slot. */
export function skybridgeCirculation(rooms: readonly ModuleRect[]) {
  const gap = MODULE_METRICS.passageWidth;
  const passages: ModuleRect[] = [];
  const openings: { x: number; y: number; axis: 'horizontal' | 'vertical' }[] = [];
  const ordered = [...rooms].sort((a, b) => a.y - b.y || a.x - b.x);
  for (let i = 0; i < ordered.length; i++)
    for (let j = i + 1; j < ordered.length; j++) {
      const a = ordered[i]!,
        b = ordered[j]!;
      const west = a.x < b.x ? a : b,
        east = west === a ? b : a;
      const top = Math.max(a.y, b.y),
        bottom = Math.min(a.y + a.height, b.y + b.height);
      if (east.x - west.x - west.width === gap && bottom - top >= gap) {
        const x = west.x + west.width,
          y = top + (bottom - top - gap) / 2;
        passages.push({ x, y, width: gap, height: gap });
        openings.push({ x, y, axis: 'vertical' }, { x: x + gap, y, axis: 'vertical' });
      }
      const left = Math.max(a.x, b.x),
        right = Math.min(a.x + a.width, b.x + b.width);
      if (b.y - a.y - a.height === gap && right - left >= gap) {
        const x = left + (right - left - gap) / 2,
          y = a.y + a.height;
        passages.push({ x, y, width: gap, height: gap });
        openings.push({ x, y, axis: 'horizontal' }, { x, y: y + gap, axis: 'horizontal' });
      }
    }
  return { passages, openings };
}
