import { MAP_LIMITS } from './map-contract.js';
import type { ModuleMapDocument, ModuleMaterial, OfficeSlot } from './module-contract.js';
import { officeSlotKey } from './module-contract.js';
import {
  meetsReservedWing,
  moduleBounds,
  moduleSlotBounds,
  modulesOverlap,
  officeExpansionPassages,
} from './module-geometry.js';
import type { WorldDocument } from './world-contract.js';
import { projectMap } from './map-geometry.js';
import { hasObjectSupport } from './world-object-placement.js';
import type { MeetingRoom } from '../local/room-contract.js';
import { addMeetingPreset } from './meeting-preset.js';

/** A finish edits the module source only; all spatial/content values remain owned elsewhere. */
export function setModuleMaterial(
  world: WorldDocument,
  areaId: string,
  material: ModuleMaterial
): WorldDocument {
  if (world.map.version === 1) throw new Error('Room styles require a modular layout.');
  if (!world.map.modules.some((module) => module.area.id === areaId))
    throw new Error('This module is no longer available.');
  return {
    ...world,
    map: {
      ...world.map,
      modules: world.map.modules.map((module) =>
        module.area.id === areaId ? { ...module, material } : module
      ),
    },
  };
}

/** Eligible cardinal neighbors only; the meeting wing never becomes an office ghost. */
export function officeExpansionSlots(source: ModuleMapDocument): OfficeSlot[] {
  if (source.modules.length >= MAP_LIMITS.areas) return [];
  const occupied = source.modules.map((module) => moduleBounds(module, source.version));
  const candidates = new Map<string, OfficeSlot>();
  const origins = source.modules.flatMap(({ slot }) =>
    slot.type === 'office'
      ? [slot]
      : slot.type === 'lobby'
        ? [
            { type: 'office' as const, column: 0, row: 0 },
            { type: 'office' as const, column: 1, row: 0 },
            ...(source.version >= 4
              ? [
                  { type: 'office' as const, column: 0, row: 1 },
                  { type: 'office' as const, column: 1, row: 1 },
                ]
              : []),
          ]
        : []
  );
  for (const origin of origins) {
    for (const [dx, dy] of [
      [-1, 0],
      [1, 0],
      [0, -1],
      [0, 1],
    ] as const) {
      const slot: OfficeSlot = { type: 'office', column: origin.column + dx, row: origin.row + dy };
      if (candidates.has(officeSlotKey(slot))) continue;
      try {
        const rect = moduleSlotBounds(slot, source.version);
        if (
          !meetsReservedWing(rect, source.version) &&
          !occupied.some((other) => modulesOverlap(rect, other))
        ) {
          // A valid saved lattice can be close to its budget. Do not expose a
          // ghost whose derived preview would throw during scene rendering.
          if (source.version >= 4) officeExpansionPassages(source, slot);
          candidates.set(officeSlotKey(slot), slot);
        }
      } catch {
        // Coordinate/floor-budget neighbors are not actionable previews.
      }
    }
  }
  return [...candidates.values()].sort((a, b) => a.row - b.row || a.column - b.column);
}

/** Commit through the caller's existing world history; no implicit Save or resident creation. */
export function addOfficeModule(
  world: WorldDocument,
  slot: OfficeSlot,
  name: string,
  id: string
): WorldDocument {
  if (world.map.version === 1) throw new Error('Whole-office editing requires a modular layout.');
  const label = name.trim();
  if (!label) throw new Error('Enter an office name.');
  if (
    !officeExpansionSlots(world.map).some(
      (candidate) => officeSlotKey(candidate) === officeSlotKey(slot)
    )
  )
    throw new Error('This office slot is no longer available. Choose another preview.');
  return {
    ...world,
    map: {
      ...world.map,
      modules: [
        ...world.map.modules,
        {
          area: { id, name: label, binding: { type: 'personal', identityId: null } },
          slot,
          material: 'workshop',
        },
      ],
    },
  };
}

/** Meeting use shares the exact prospective grid slot and ordinary area creation. */
export function addGridMeetingModule(
  world: WorldDocument,
  slot: OfficeSlot,
  room: MeetingRoom,
  id: string
): WorldDocument {
  if (world.map.version !== 8) throw new Error('Convert to unified areas first.');
  if (room.retired) throw new Error('A retired room cannot be placed.');
  if (
    world.map.modules.some(
      ({ area }) => area.binding.type === 'meeting' && area.binding.roomId === room.id
    )
  )
    throw new Error('This meeting room already has a space in this layout.');
  const added = addOfficeModule(world, slot, room.name, id);
  if (added.map.version === 1) throw new Error('A modular layout is required.');
  return addMeetingPreset(
    {
      ...added,
      map: {
        ...added.map,
        modules: added.map.modules.map((module) =>
          module.area.id === id
            ? {
                ...module,
                area: { ...module.area, binding: { type: 'meeting' as const, roomId: room.id } },
              }
            : module
        ),
      },
    },
    id
  );
}

/** A source-only removal candidate; contents and canonical room membership stay intact. */
export function previewModuleRemoval(world: WorldDocument, areaId: string) {
  if (world.map.version === 1) throw new Error('Whole-office editing requires a modular layout.');
  const module = world.map.modules.find((value) => value.area.id === areaId);
  if (!module) throw new Error('This module is no longer available.');
  if (module.slot.type === 'lobby')
    return { next: world, blockedObjects: [], reason: 'The central Lobby is required.' };
  const next = {
    ...world,
    map: { ...world.map, modules: world.map.modules.filter((value) => value.area.id !== areaId) },
  };
  const geometry = projectMap(next.map);
  // Inspect the whole candidate: removing a module can also remove adjacent
  // circulation or a partition supporting an object outside the selected room.
  const blockedObjects = world.objects.filter((object) => !hasObjectSupport(geometry, object));
  return {
    next,
    blockedObjects,
    reason: blockedObjects.length
      ? 'Move or explicitly remove the affected placements first.'
      : undefined,
  };
}

export function removeModule(world: WorldDocument, areaId: string): WorldDocument {
  const preview = previewModuleRemoval(world, areaId);
  if (preview.reason) throw new Error(preview.reason);
  return preview.next;
}
