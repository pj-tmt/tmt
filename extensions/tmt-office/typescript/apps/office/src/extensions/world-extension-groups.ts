import { indexFloor } from '../world-map/floor-index.js';
import { mapGeometry } from '../world-map/map-source.js';
import { objectArea } from '../world-map/object-area.js';
import type { WorldDocument } from '../world-map/world-contract.js';
import type { BoundExtension } from './extension-binding.js';

export interface WorldExtensionGroup {
  areaId: string | null | undefined;
  label: string;
  entries: BoundExtension[];
}

/** Physical placement determines discovery, never resource ownership or permissions. */
export function worldExtensionGroups(world: WorldDocument, entries: readonly BoundExtension[]) {
  const map = mapGeometry(world.map);
  const floor = indexFloor(map.floor);
  const areas = new Map(map.areas.map((area) => [area.id, area.name]));
  const objects = new Map(world.objects.map((object) => [object.id, object]));
  const groups = new Map<string | null | undefined, WorldExtensionGroup>();
  for (const entry of entries) {
    const object = objects.get(entry.instance.id);
    if (!object) continue;
    const areaId = objectArea(floor, object);
    const group = groups.get(areaId) ?? {
      areaId,
      label:
        areaId === null
          ? 'Common floor'
          : areaId === undefined
            ? 'Outside current floor'
            : (areas.get(areaId) ?? 'Unavailable area'),
      entries: [],
    };
    group.entries.push(entry);
    groups.set(areaId, group);
  }
  return [...groups.values()];
}
