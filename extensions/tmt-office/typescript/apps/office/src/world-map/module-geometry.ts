import { MAP_LIMITS } from './map-contract.js';
import type { FloorSpan, MapDocument, MapEdge } from './map-contract.js';
import type { ModuleMapDocument, ModuleSlot, OfficeModule, OfficeSlot } from './module-contract.js';
import { canonicalMapDraft } from './map-draft.js';
import { subtractFloorInterval } from './free-floor.js';

import { MODULE_METRICS } from './module-metrics.js';
import { compactCirculation, skybridgeCirculation } from './compact-circulation.js';
export { MODULE_METRICS } from './module-metrics.js';
export interface ModuleRect {
  x: number;
  y: number;
  width: number;
  height: number;
}
const right = (rect: ModuleRect) => rect.x + rect.width;
const bottom = (rect: ModuleRect) => rect.y + rect.height;
export function modulesOverlap(a: ModuleRect, b: ModuleRect): boolean {
  return a.x < right(b) && right(a) > b.x && a.y < bottom(b) && bottom(a) > b.y;
}

export function moduleBounds(
  module: OfficeModule,
  version: ModuleMapDocument['version']
): ModuleRect {
  const purpose = { lobby: 'lobby', office: 'personal', meeting: 'meeting' }[module.slot.type];
  const unified =
    version === 8 && module.slot.type === 'office' && module.area.binding.type === 'meeting';
  if (
    (version === 8 && module.slot.type === 'meeting') ||
    (!unified && module.area.binding.type !== purpose)
  )
    throw new Error('Office module slot does not match its purpose.');
  return moduleSlotBounds(module.slot, version);
}

export function moduleSlotBounds(slot: ModuleSlot, version: number): ModuleRect {
  const m = MODULE_METRICS;
  let rect: ModuleRect;
  if (slot.type === 'lobby')
    rect = {
      x: 0,
      y: 0,
      width: m.lobbyWidth,
      height: version >= 4 ? 2 * m.roomHeight + m.passageWidth : m.roomHeight,
    };
  else if (slot.type === 'office')
    rect = {
      x: slot.column * m.columnStep,
      y: slot.row * m.rowStep,
      width: m.roomWidth,
      height: m.roomHeight,
    };
  else
    rect = {
      x: m.meetingX + (version >= 6 ? 2 * m.passageWidth : 0),
      y: slot.index * (version === 5 ? m.roomHeight : m.rowStep),
      width: m.roomWidth,
      height: m.roomHeight,
    };
  if (
    ![rect.x, rect.y].every(Number.isSafeInteger) ||
    rect.x < -MAP_LIMITS.coordinate ||
    rect.y < -MAP_LIMITS.coordinate ||
    right(rect) > MAP_LIMITS.coordinate ||
    bottom(rect) > MAP_LIMITS.coordinate
  )
    throw new Error('Office module exceeds coordinate bounds.');
  return rect;
}

export function meetsReservedWing(rect: ModuleRect, version = 4): boolean {
  if (version === 8) return false;
  const m = MODULE_METRICS;
  return modulesOverlap(rect, {
    x: m.meetingX - m.passageWidth,
    y: 0,
    width: m.roomWidth + m.passageWidth + (version >= 6 ? 2 * m.passageWidth : 0),
    height: MAP_LIMITS.coordinate,
  });
}

export interface ModuleConnection {
  passage: ModuleRect;
  /** Ordered west-to-east or north-to-south, independent of source ordering. */
  openings: readonly [MapEdge, MapEdge];
}

/** One connection definition supplies physical openings and their visual assembly. */
export function moduleConnections(
  modules: readonly Pick<OfficeModule, 'slot'>[],
  grid = false,
  version = grid ? 3 : 2
): ModuleConnection[] {
  const size = MODULE_METRICS.passageWidth;
  const bounds = modules
    .filter((module) => module.slot.type !== 'meeting')
    .map(({ slot }) => moduleSlotBounds(slot, version));
  const result: ModuleConnection[] = [];
  for (const [index, a] of bounds.entries()) {
    for (const b of bounds.slice(0, index)) {
      const [left, next] = a.x < b.x ? [a, b] : [b, a];
      const overlapY = Math.min(bottom(left), bottom(next)) - Math.max(left.y, next.y);
      if (next.x - right(left) === size && overlapY >= size) {
        const y = Math.max(left.y, next.y) + Math.floor((overlapY - size) / 2);
        result.push({
          passage: {
            x: right(left),
            y: grid ? Math.max(left.y, next.y) : y,
            width: size,
            height: grid ? overlapY : size,
          },
          openings: [
            { x: right(left), y, axis: 'vertical' },
            { x: next.x, y, axis: 'vertical' },
          ],
        });
      }
      const [above, below] = a.y < b.y ? [a, b] : [b, a];
      const overlapX = Math.min(right(above), right(below)) - Math.max(above.x, below.x);
      if (below.y - bottom(above) === size && overlapX >= size) {
        const x = Math.max(above.x, below.x) + Math.floor((overlapX - size) / 2);
        result.push({
          passage: {
            x: grid ? Math.max(above.x, below.x) : x,
            y: bottom(above),
            width: grid ? overlapX : size,
            height: size,
          },
          openings: [
            { x, y: bottom(above), axis: 'horizontal' },
            { x, y: below.y, axis: 'horizontal' },
          ],
        });
      }
    }
  }
  return result;
}

/** Internal projection of validated main bounds, always including the primary Lobby. */
function centralGridCirculation(rooms: readonly ModuleRect[], compact = false, bridges = false) {
  const m = MODULE_METRICS;
  const left = Math.min(...rooms.map((rect) => rect.x));
  const rightEdge = Math.max(...rooms.map(right));
  const top = Math.min(...rooms.map((rect) => rect.y));
  const bottomEdge = Math.max(...rooms.map(bottom));
  const exclusions = [
    moduleSlotBounds({ type: 'lobby' }, 4),
    {
      x: m.meetingX - m.passageWidth,
      y: 0,
      width: m.roomWidth + m.passageWidth + (bridges ? 2 * m.passageWidth : 0),
      height: MAP_LIMITS.coordinate,
    },
  ];
  const passages: ModuleRect[] = [];
  const rows = new Map<number, { start: number; end: number }[]>();
  let tiles = rooms.reduce((sum, room) => sum + room.width * room.height, 0);
  let spans = rooms.reduce((sum, room) => sum + room.height, 0);
  const skybridges = bridges ? skybridgeCirculation(rooms) : undefined;
  const required = skybridges?.passages ?? (compact ? compactCirculation(rooms) : []);
  for (let y = top; y < bottomEdge; y++) {
    const localY = ((y % m.rowStep) + m.rowStep) % m.rowStep;
    let runs: { start: number; end: number }[] = [];
    if (compact) {
      const intervals = required
        .filter((rect) => y >= rect.y && y < bottom(rect))
        .map((rect) => ({ start: rect.x, end: right(rect) }))
        .sort((a, b) => a.start - b.start);
      for (const interval of intervals) {
        const last = runs.at(-1);
        if (last && interval.start <= last.end) last.end = Math.max(last.end, interval.end);
        else runs.push(interval);
      }
    } else if (localY >= m.roomHeight) runs.push({ start: left, end: rightEdge });
    else
      for (
        let column = Math.floor(left / m.columnStep);
        column <= Math.floor(rightEdge / m.columnStep);
        column++
      ) {
        const start = Math.max(left, column * m.columnStep + m.roomWidth);
        const end = Math.min(rightEdge, (column + 1) * m.columnStep);
        if (start < end) runs.push({ start, end });
      }
    for (const exclusion of exclusions) {
      if (y < exclusion.y || y >= bottom(exclusion)) continue;
      runs = subtractFloorInterval(runs, exclusion.x, right(exclusion), 1);
    }
    for (const { start, end } of runs) {
      tiles += end - start;
      spans++;
      if (tiles > MAP_LIMITS.tiles || spans > MAP_LIMITS.spans)
        throw new Error('Office grid exceeds floor budgets.');
      passages.push({ x: start, y, width: end - start, height: 1 });
    }
    rows.set(y, runs);
  }
  const contains = (x: number, y: number) =>
    rows.get(y)?.some((run) => run.start <= x && x < run.end) ?? false;
  const offsets = Array.from({ length: m.passageWidth }, (_, i) => i);
  const openings: MapEdge[] = skybridges?.openings ?? [];
  if (bridges) return { passages, openings };
  for (const room of rooms) {
    const x = room.x + (room.width - m.passageWidth) / 2;
    const y = room.y + (room.height - m.passageWidth) / 2;
    for (const [edgeX, outsideX] of [
      [room.x, room.x - 1],
      [right(room), right(room)],
    ] as const)
      if (offsets.every((i) => contains(outsideX, y + i)))
        openings.push({ x: edgeX, y, axis: 'vertical' });
    for (const [edgeY, outsideY] of [
      [room.y, room.y - 1],
      [bottom(room), bottom(room)],
    ] as const)
      if (offsets.every((i) => contains(x + i, outsideY)))
        openings.push({ x, y: edgeY, axis: 'horizontal' });
  }
  return { passages, openings };
}

/** Unsaved circulation preview derived from the same geometry as the accepted room. */
export function officeExpansionPassages(source: ModuleMapDocument, slot: OfficeSlot): ModuleRect[] {
  const room = moduleSlotBounds(slot, source.version);
  if (source.version < 4 || source.version === 8)
    return moduleConnections([...source.modules, { slot }], source.version === 3, source.version)
      .filter(({ openings }) =>
        openings.some((edge) =>
          edge.axis === 'vertical'
            ? (edge.x === room.x || edge.x === right(room)) &&
              edge.y >= room.y &&
              edge.y < bottom(room)
            : (edge.y === room.y || edge.y === bottom(room)) &&
              edge.x >= room.x &&
              edge.x < right(room)
        )
      )
      .map(({ passage }) => passage);
  const rooms = source.modules
    .filter((module) => module.slot.type !== 'meeting')
    .map((module) => moduleBounds(module, source.version));
  const before = new Map<number, ModuleRect[]>();
  for (const rect of centralGridCirculation(rooms, source.version >= 5, source.version >= 6)
    .passages) {
    const row = before.get(rect.y) ?? [];
    row.push(rect);
    before.set(rect.y, row);
  }
  const result: ModuleRect[] = [];
  for (const rect of centralGridCirculation(
    [...rooms, room],
    source.version >= 5,
    source.version >= 6
  ).passages) {
    let intervals = [{ start: rect.x, end: right(rect) }];
    for (const old of before.get(rect.y) ?? [])
      intervals = subtractFloorInterval(intervals, old.x, right(old), 1);
    result.push(
      ...intervals.map(({ start, end }) => ({ x: start, y: rect.y, width: end - start, height: 1 }))
    );
  }
  return result;
}

/** The stable meeting spine is shared by saved geometry and construction previews. */
export function meetingCirculation(slots: readonly ModuleSlot[], version: number) {
  // Independent islands retain their fixed slots but have no common floor or portals.
  if (version >= 7) return { passages: [] as ModuleRect[], openings: [] as MapEdge[] };
  const m = MODULE_METRICS;
  const meetings = slots
    .filter((slot) => slot.type === 'meeting')
    .map((slot) => moduleSlotBounds(slot, version));
  if (!meetings.length) return { passages: [] as ModuleRect[], openings: [] as MapEdge[] };
  const roomDoorY = (m.roomHeight - m.passageWidth) / 2;
  const y = version >= 4 ? m.roomHeight : roomDoorY;
  const first = Math.min(y, ...meetings.map((rect) => rect.y + roomDoorY));
  const last = Math.max(y, ...meetings.map((rect) => rect.y + roomDoorY));
  return {
    passages: [
      { x: m.lobbyWidth, y, width: m.meetingX - m.lobbyWidth, height: m.passageWidth },
      {
        x: m.meetingX - m.passageWidth,
        y: first,
        width: m.passageWidth,
        height: last + m.passageWidth - first,
      },
      ...(version >= 6
        ? meetings.map((rect) => ({
            x: m.meetingX,
            y: rect.y + roomDoorY,
            width: rect.x - m.meetingX,
            height: m.passageWidth,
          }))
        : []),
    ],
    openings: [
      { x: m.lobbyWidth, y, axis: 'vertical' as const },
      ...meetings.map((rect) => ({ x: rect.x, y: rect.y + roomDoorY, axis: 'vertical' as const })),
    ],
  };
}

/** Stateless projection into the existing sparse-floor validator and renderer. */
export function projectModules(source: ModuleMapDocument): MapDocument {
  const m = MODULE_METRICS,
    size = m.passageWidth;
  const bounds = source.modules.map((module) => moduleBounds(module, source.version));
  const lobby = source.modules.filter((module) => module.slot.type === 'lobby');
  if (lobby.length !== 1 || lobby[0]!.area.id !== source.primaryLobbyId)
    throw new Error('Office requires one primary Lobby module.');
  for (const [index, module] of source.modules.entries()) {
    if (
      (module.slot.type === 'office' && meetsReservedWing(bounds[index]!, source.version)) ||
      bounds.slice(0, index).some((other) => modulesOverlap(other, bounds[index]!))
    )
      throw new Error('Office module overlaps another module or the reserved meeting wing.');
  }
  const floor: FloorSpan[] = [],
    doors = new Map<string, MapEdge>();
  const common = new Map<number, { start: number; end: number }[]>();
  function passage(rect: ModuleRect) {
    for (let y = rect.y; y < bottom(rect); y++) {
      const ranges = common.get(y) ?? [];
      ranges.push({ start: rect.x, end: right(rect) });
      common.set(y, ranges);
    }
  }
  function door(x: number, y: number, axis: MapEdge['axis']) {
    for (let offset = 0; offset < size; offset++) {
      const edge = {
        x: x + (axis === 'horizontal' ? offset : 0),
        y: y + (axis === 'vertical' ? offset : 0),
        axis,
      };
      doors.set(`${edge.x},${edge.y},${axis}`, edge);
    }
  }
  for (const [index, module] of source.modules.entries()) {
    const rect = bounds[index]!;
    for (let y = rect.y; y < bottom(rect); y++)
      floor.push({ y, start: rect.x, end: right(rect), areaId: module.area.id });
  }
  if (source.version === 8) {
    for (const connection of moduleConnections(source.modules, false, 8)) {
      passage(connection.passage);
      for (const edge of connection.openings) door(edge.x, edge.y, edge.axis);
    }
  } else if (source.version >= 4) {
    const main = bounds.filter((_, index) => source.modules[index]!.slot.type !== 'meeting');
    const grid = centralGridCirculation(main, source.version >= 5, source.version >= 6);
    for (const rect of grid.passages) passage(rect);
    for (const edge of grid.openings) door(edge.x, edge.y, edge.axis);
  } else {
    for (const connection of moduleConnections(source.modules, source.version === 3)) {
      passage(connection.passage);
      for (const edge of connection.openings) door(edge.x, edge.y, edge.axis);
    }
  }
  if (source.version === 3) {
    const candidates = new Map<string, { x: number; y: number }>();
    for (const [index, rect] of bounds.entries()) {
      if (source.modules[index]!.slot.type === 'meeting') continue;
      for (const x of [rect.x - size, right(rect)])
        for (const y of [rect.y - size, bottom(rect)]) candidates.set(`${x},${y}`, { x, y });
    }
    const contains = (x: number, y: number) =>
      common.get(y)?.some((range) => range.start <= x && x < range.end) ?? false;
    const offsets = Array.from({ length: size }, (_, i) => i);
    const joins = [...candidates.values()].filter(
      ({ x, y }) =>
        [
          offsets.every((i) => contains(x + i, y - 1)),
          offsets.every((i) => contains(x + i, y + size)),
          offsets.every((i) => contains(x - 1, y + i)),
          offsets.every((i) => contains(x + size, y + i)),
        ].filter(Boolean).length >= 2
    );
    for (const { x, y } of joins) passage({ x, y, width: size, height: size });
  }
  const wing = meetingCirculation(
    source.modules.map((module) => module.slot),
    source.version
  );
  for (const rect of wing.passages) passage(rect);
  for (const edge of wing.openings) door(edge.x, edge.y, edge.axis);
  for (const [y, ranges] of common) {
    const merged: { start: number; end: number }[] = [];
    for (const range of ranges.sort((a, b) => a.start - b.start)) {
      const previous = merged.at(-1);
      if (previous && range.start <= previous.end) previous.end = Math.max(previous.end, range.end);
      else merged.push({ ...range });
    }
    floor.push(...merged.map((range) => ({ y, ...range, areaId: null })));
  }
  if (
    floor.length > MAP_LIMITS.spans ||
    floor.reduce((sum, span) => sum + span.end - span.start, 0) > MAP_LIMITS.tiles
  )
    throw new Error('Office grid exceeds floor budgets.');
  return canonicalMapDraft({
    version: 1,
    primaryLobbyId: source.primaryLobbyId,
    areas: source.modules.map((module) => module.area),
    floor,
    doors: [...doors.values()],
  });
}
