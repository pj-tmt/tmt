import type { SceneRect } from './office-geometry.js';
import { MODULE_METRICS } from '../world-map/module-metrics.js';

/** Standard visible connector span before the common depth projection. */
export const PLATFORM_BRIDGE_LENGTH = 24;

/** Expand empty bands between platforms, never room interiors. Both directions
 * share the same monotone transform, including negative coordinates and picking.
 * Normalize around the Lobby origin so adding a northern room does not shift it.
 */
function spacedAxis(intervals: readonly [number, number][]) {
  const merged: [number, number][] = [];
  for (const [start, end] of [...intervals].sort((a, b) => a[0] - b[0])) {
    const previous = merged.at(-1);
    if (previous && start <= previous[1]) previous[1] = Math.max(previous[1], end);
    else merged.push([start, end]);
  }
  const gaps = merged.slice(1).map(([end], index) => {
    const start = merged[index]![1];
    const length = end - start;
    // Longer routed circulation is not compressed into a short connector.
    return { start, end, length, extra: Math.max(0, PLATFORM_BRIDGE_LENGTH - length) };
  });
  const offset = (value: number) =>
    gaps.reduce(
      (sum, gap) =>
        sum + (Math.max(0, Math.min(value, gap.end) - gap.start) * gap.extra) / gap.length,
      0
    );
  const zero = offset(0);
  const project = (value: number) => value + offset(value) - zero;
  function unproject(value: number) {
    let extra = 0;
    const shifted = value + zero;
    for (const gap of gaps) {
      const start = gap.start + extra;
      if (shifted < start) return shifted - extra;
      const span = gap.length + gap.extra;
      if (shifted <= start + span) return gap.start + ((shifted - start) * gap.length) / span;
      extra += gap.extra;
    }
    return shifted - extra;
  }
  return { project, unproject };
}

/** A fixed lattice, not an envelope of occupied rooms. The Lobby spans two
 * cells on each axis, including their band; its interior stays one platform. */
function latticeAxis(size: number, step: number) {
  const displayStep = size + PLATFORM_BRIDGE_LENGTH;
  function periodic(value: number, from: number, to: number) {
    const cell = Math.floor(value / from);
    const local = value - cell * from;
    return (
      cell * to + Math.min(local, size) + (Math.max(0, local - size) * (to - size)) / (from - size)
    );
  }
  return {
    project(value: number) {
      return periodic(value, step, displayStep);
    },
    unproject(value: number) {
      return periodic(value, displayStep, step);
    },
  };
}

export function platformProjection(rooms: Iterable<SceneRect>, depth: number, version = 6) {
  const { meetingX, passageWidth, roomWidth, roomHeight, rowStep, columnStep } = MODULE_METRICS;
  const wingStart = meetingX + 2 * passageWidth;
  const inWing = (x: number, y: number) =>
    version === 7 && x >= wingStart && x <= wingStart + roomWidth && y >= 0;
  // Independent islands cannot change the main campus's gap expansion. Their
  // fixed slot transform includes empty slots, so removal never packs survivors.
  const bounds = [...rooms].filter((room) => !inWing(room.x, room.y));
  const xIntervals: [number, number][] = bounds.map((room) => [room.x, room.x + room.width]);
  // Reserve the full wing even while empty. A northern office's gap must not
  // stretch island interiors or disappear when a meeting is subsequently added.
  if (version === 7) xIntervals.push([wingStart, wingStart + roomWidth]);
  const x = version === 8 ? latticeAxis(roomWidth, columnStep) : spacedAxis(xIntervals);
  const y =
    version === 8
      ? latticeAxis(roomHeight, rowStep)
      : spacedAxis(bounds.map((room) => [room.y, room.y + room.height]));
  const islandStep = roomHeight + PLATFORM_BRIDGE_LENGTH;
  function islandY(value: number, inverse = false) {
    const sourceStep = inverse ? islandStep : rowStep;
    const targetStep = inverse ? rowStep : islandStep;
    const slot = Math.floor(value / sourceStep);
    const local = value - slot * sourceStep;
    return (
      slot * targetStep +
      Math.min(local, roomHeight) +
      (Math.max(0, local - roomHeight) * (targetStep - roomHeight)) / (sourceStep - roomHeight)
    );
  }
  function projectGround(point: { x: number; y: number }) {
    return {
      x: x.project(point.x),
      y: (inWing(point.x, point.y) ? islandY(point.y) : y.project(point.y)) * depth,
    };
  }
  function unprojectGround(point: { x: number; y: number }) {
    const groundX = x.unproject(point.x);
    const groundY = point.y / depth;
    return {
      x: groundX,
      y: inWing(groundX, groundY) ? islandY(groundY, true) : y.unproject(groundY),
    };
  }
  const projectGroundRect = (rect: SceneRect, _owner?: string | null): SceneRect => {
    const start = projectGround(rect);
    const end = projectGround({ x: rect.x + rect.width, y: rect.y + rect.height });
    return { ...start, width: end.x - start.x, height: end.y - start.y };
  };
  return {
    version,
    uprightAnchorOffset: (height: number) => height,
    projectGround,
    unprojectGround,
    projectGroundRect,
    projectModuleFloor: projectGroundRect,
    projectUpright: (rect: SceneRect): SceneRect => {
      const anchor = projectGround({ x: rect.x, y: rect.y + rect.height });
      return { ...rect, x: anchor.x, y: anchor.y - rect.height };
    },
    projectWallGround: projectGround,
    unprojectWallGround: unprojectGround,
  };
}
