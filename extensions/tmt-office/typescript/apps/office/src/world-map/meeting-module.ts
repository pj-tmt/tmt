import { MAP_LIMITS } from './map-contract.js';
import type { MeetingSlot, ModuleMapDocument } from './module-contract.js';
import { meetingCirculation, moduleSlotBounds } from './module-geometry.js';
import type { ModuleRect } from './module-geometry.js';
import { mapGeometry } from './map-source.js';
import { subtractFloorInterval } from './free-floor.js';
import { decodeWorldDocument } from './world-contract.js';
import type { WorldDocument } from './world-contract.js';
import type { MeetingRoom } from '../local/room-contract.js';
import { addMeetingPreset } from './meeting-preset.js';

/** Only the new common floor is previewed; saved circulation is never overpainted. */
export function meetingExpansionPassages(
  source: ModuleMapDocument,
  slot: MeetingSlot
): ModuleRect[] {
  const common = new Map<number, { start: number; end: number }[]>();
  for (const row of mapGeometry(source).floor) {
    if (row.areaId !== null) continue;
    const ranges = common.get(row.y) ?? [];
    ranges.push(row);
    common.set(row.y, ranges);
  }
  const result: ModuleRect[] = [];
  const wing = meetingCirculation(
    [...source.modules.map((module) => module.slot), slot],
    source.version
  );
  for (const rect of wing.passages) {
    for (let y = rect.y; y < rect.y + rect.height; y++) {
      let ranges = [{ start: rect.x, end: rect.x + rect.width }];
      for (const old of common.get(y) ?? [])
        ranges = subtractFloorInterval(ranges, old.start, old.end, 1);
      for (const range of ranges) {
        result.push({ x: range.start, y, width: range.end - range.start, height: 1 });
        const row = common.get(y) ?? [];
        row.push(range);
        common.set(y, row);
      }
    }
  }
  return result;
}

/** Append below the last occupied slot. Removing another room never repacks it. */
export function nextMeetingSlot(source: ModuleMapDocument): MeetingSlot | undefined {
  if (source.version === 8) return;
  if (source.modules.length >= MAP_LIMITS.areas) return;
  const index =
    1 +
    Math.max(
      -1,
      ...source.modules.flatMap(({ slot }) => (slot.type === 'meeting' ? [slot.index] : []))
    );
  const slot: MeetingSlot = { type: 'meeting', index };
  try {
    const bounds = moduleSlotBounds(slot, source.version);
    const additional = meetingExpansionPassages(source, slot);
    const existing = mapGeometry(source).floor.reduce((sum, row) => sum + row.end - row.start, 0);
    if (
      existing +
        bounds.width * bounds.height +
        additional.reduce((sum, rect) => sum + rect.width * rect.height, 0) >
      MAP_LIMITS.tiles
    )
      return;
    return slot;
  } catch {
    return;
  }
}

/** A stored canonical room is attached to the existing world draft, never created here. */
export function addMeetingModule(
  world: WorldDocument,
  slot: MeetingSlot,
  room: MeetingRoom,
  areaId: string
): WorldDocument {
  if (world.map.version === 1) throw new Error('Meeting modules require a modular layout.');
  if (room.retired) throw new Error('A retired room cannot be placed.');
  if (nextMeetingSlot(world.map)?.index !== slot.index)
    throw new Error(
      'This meeting slot is no longer available. Keep the room and choose another slot.'
    );
  if (
    world.map.modules.some(
      ({ area }) => area.binding.type === 'meeting' && area.binding.roomId === room.id
    )
  )
    throw new Error('This meeting room already has a space in this layout.');
  const next = decodeWorldDocument({
    ...world,
    map: {
      ...world.map,
      modules: [
        ...world.map.modules,
        {
          area: { id: areaId, name: room.name, binding: { type: 'meeting', roomId: room.id } },
          slot,
          material: 'workshop',
        },
      ],
    },
  });
  return addMeetingPreset(next, areaId);
}
