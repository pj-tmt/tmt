import type { FloorSpan } from '../world-map/map-contract.js';
import { AVATAR_LAYOUT } from '../profiles/avatar-layout.js';
import type { SceneRect } from './office-geometry.js';
import {
  freeFloorRows,
  floorRectangleIntervals,
  subtractFloorInterval,
} from '../world-map/free-floor.js';

const WIDTH = Math.ceil(AVATAR_LAYOUT.width) + 4;
const HEIGHT = Math.ceil(AVATAR_LAYOUT.height) + 4;
export const AREA_ACTOR_PREVIEW_LIMIT = 6;

function actorRect(rect: SceneRect): SceneRect {
  return {
    x: rect.x + (WIDTH - AVATAR_LAYOUT.width) / 2,
    y: rect.y + HEIGHT - AVATAR_LAYOUT.height - 1,
    width: AVATAR_LAYOUT.width,
    height: AVATAR_LAYOUT.height,
  };
}

/** Bounded visual slots, not saved positions or membership. Never place an actor in a void,
 * across an area boundary, or over floor furniture. Small areas may have no preview. */
export function areaActorSlots(
  rows: FloorSpan[],
  anchor: { x: number; y: number },
  obstacles: SceneRect[],
  preferred?: (rect: SceneRect) => boolean
): SceneRect[] {
  const freeRows = freeFloorRows(rows, obstacles, WIDTH);
  const topRows = [...freeRows.keys()].sort(
    (a, b) => Math.abs(a + HEIGHT / 2 - anchor.y) - Math.abs(b + HEIGHT / 2 - anchor.y) || a - b
  );
  const occupied: SceneRect[] = [];
  // Visibility is a preference, never permission to cross furniture or floor.
  // Retain a bounded fallback for densely furnished areas with no clear view.
  for (const requirePreferred of preferred ? [true, false] : [false]) {
    for (const y of topRows) {
      let intervals = floorRectangleIntervals(freeRows, y, HEIGHT, WIDTH);
      // Remove previously reserved rectangles, including their nameplate/spacing margin.
      for (const rect of occupied) {
        if (y >= rect.y + rect.height || y + HEIGHT <= rect.y) continue;
        intervals = subtractFloorInterval(intervals, rect.x, rect.x + rect.width, WIDTH);
      }
      while (intervals.length && occupied.length < AREA_ACTOR_PREVIEW_LIMIT) {
        const candidates = intervals
          .flatMap((part) => {
            const center = Math.max(
              part.start,
              Math.min(part.end - WIDTH, Math.round(anchor.x - WIDTH / 2))
            );
            const distance = Math.max(center - part.start, part.end - WIDTH - center);
            for (let step = 0; step <= distance; step++) {
              for (const x of step ? [center - step, center + step] : [center]) {
                if (x < part.start || x + WIDTH > part.end) continue;
                if (
                  !requirePreferred ||
                  preferred!(actorRect({ x, y, width: WIDTH, height: HEIGHT }))
                )
                  return [{ part, x }];
              }
            }
            return [];
          })
          .sort(
            (a, b) =>
              Math.abs(a.x + WIDTH / 2 - anchor.x) - Math.abs(b.x + WIDTH / 2 - anchor.x) ||
              a.x - b.x
          );
        if (!candidates.length) break;
        const { part, x } = candidates[0]!;
        occupied.push({ x, y, width: WIDTH, height: HEIGHT });
        intervals = intervals
          .filter((value) => value !== part)
          .concat(
            [
              { start: part.start, end: x },
              { start: x + WIDTH, end: part.end },
            ].filter((value) => value.end - value.start >= WIDTH)
          );
      }
      if (occupied.length === AREA_ACTOR_PREVIEW_LIMIT) break;
    }
    if (occupied.length === AREA_ACTOR_PREVIEW_LIMIT) break;
  }
  return occupied.map(actorRect);
}
