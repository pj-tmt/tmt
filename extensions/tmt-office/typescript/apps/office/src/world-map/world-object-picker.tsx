import type { CatalogPack } from '../props/prop-contract.js';
import { resolvePlacedProp } from '../props/prop-contract.js';
import { indexFloor } from './floor-index.js';
import { mapGeometry } from './map-source.js';
import { objectArea } from './object-area.js';
import type { WorldDocument } from './world-contract.js';

/** A room-first view of the existing placements, never a filtered repair inventory. */
export function WorldObjectPicker({
  world,
  catalog,
  areaId,
  value,
  select,
}: {
  world: WorldDocument;
  catalog: CatalogPack[];
  areaId: string | undefined;
  value: string | undefined;
  select: (id: string) => void;
}) {
  const map = mapGeometry(world.map);
  const floor = indexFloor(map.floor);
  const groups = new Map<string | null | undefined, { value: string; label: string }[]>();
  world.objects.forEach((object, index) => {
    const owner = objectArea(floor, object);
    const entries = groups.get(owner) ?? [];
    entries.push({
      value: object.id,
      label: `${index + 1} · ${resolvePlacedProp(catalog, object.placement)?.definition.label ?? 'Unavailable prop'}`,
    });
    groups.set(owner, entries);
  });
  const current = areaId === '' ? null : areaId;
  const order = [...map.areas.map((area) => area.id), null, undefined];
  if (areaId !== undefined) order.sort((a, b) => Number(b === current) - Number(a === current));
  return (
    <label>
      Object
      <select value={value ?? ''} onChange={(event) => select(event.target.value)}>
        <option value="">Select an object</option>
        {order.map((owner) => {
          const entries = groups.get(owner);
          if (!entries?.length) return null;
          const name =
            owner === undefined
              ? 'Outside layout'
              : owner === null
                ? 'Common floor'
                : map.areas.find((area) => area.id === owner)!.name;
          return (
            <optgroup
              key={owner ?? (owner === null ? 'common' : 'outside')}
              label={`${name}${areaId !== undefined && owner === current ? ' · Current room' : ''} (${entries.length})`}
            >
              {entries.map((entry) => (
                <option key={entry.value} value={entry.value}>
                  {entry.label}
                </option>
              ))}
            </optgroup>
          );
        })}
      </select>
    </label>
  );
}
