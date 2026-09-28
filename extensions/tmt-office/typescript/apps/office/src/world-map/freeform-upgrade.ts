import { indexFloor } from './floor-index.js';
import type { MapDocument } from './map-contract.js';
import { officeExpansionSlots } from './module-authoring.js';
import type { ModuleMapDocument, OfficeModule } from './module-contract.js';
import { moduleBounds, moduleSlotBounds } from './module-geometry.js';

/** Deterministic preview only: reuse the normal eligible-slot policy. */
export function planFreeformModules(source: MapDocument) {
  const areas = source.areas.map((area) => {
    const bounds = indexFloor(source.floor.filter((span) => span.areaId === area.id)).bounds;
    if (!bounds)
      throw new Error(`Area ${area.name} has no floor. Remove the empty area before upgrading.`);
    return { area, bounds };
  });
  const lobby = areas.find(({ area }) => area.id === source.primaryLobbyId);
  if (!lobby || lobby.area.binding.type !== 'lobby')
    throw new Error('Choose a primary Lobby first.');
  if (areas.some(({ area }) => area.binding.type === 'lobby' && area.id !== lobby.area.id))
    throw new Error(
      'A modular layout has one Lobby. Consolidate extra Lobby areas before upgrading.'
    );
  const modules: OfficeModule[] = [
    { area: lobby.area, slot: { type: 'lobby' }, material: 'workshop' },
  ];
  const map: ModuleMapDocument = { version: 4, primaryLobbyId: source.primaryLobbyId, modules };
  const center = (bounds: typeof lobby.bounds) => ({
    x: bounds.x + bounds.width / 2,
    y: bounds.y + bounds.height / 2,
  });
  const origin = center(lobby.bounds);
  const target = center(moduleBounds(modules[0]!, 4));
  const ordered = areas
    .filter(({ area }) => area.id !== lobby.area.id)
    .sort(
      (a, b) =>
        a.bounds.y - b.bounds.y || a.bounds.x - b.bounds.x || a.area.id.localeCompare(b.area.id)
    );
  let meetingIndex = 0;
  for (const { area, bounds } of ordered) {
    if (area.binding.type === 'meeting') {
      modules.push({
        area,
        slot: { type: 'meeting', index: meetingIndex++ },
        material: 'workshop',
      });
      continue;
    }
    const from = center(bounds);
    const distance = (slot: ReturnType<typeof officeExpansionSlots>[number]) => {
      const to = center(moduleSlotBounds(slot, 4));
      return (
        (to.x - target.x - (from.x - origin.x)) ** 2 + (to.y - target.y - (from.y - origin.y)) ** 2
      );
    };
    const slot = officeExpansionSlots(map).sort((a, b) => distance(a) - distance(b))[0];
    if (!slot) throw new Error(`No available module slot for ${area.name}.`);
    modules.push({ area, slot, material: 'workshop' });
  }
  return {
    map,
    moves: modules.map((module) => ({
      areaId: module.area.id,
      before: areas.find(({ area }) => area.id === module.area.id)!.bounds,
      after: moduleBounds(module, 4),
    })),
  };
}
