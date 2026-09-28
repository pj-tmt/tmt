import { canonicalUuid, exactRecord } from '../contracts/record.js';
import { decodeMapArea, MAP_LIMITS } from './map-contract.js';
import type { MapArea } from './map-contract.js';

export interface OfficeSlot {
  readonly type: 'office';
  readonly column: number;
  readonly row: number;
}
export const officeSlotKey = (slot: OfficeSlot) => `${slot.column},${slot.row}`;
export interface MeetingSlot {
  readonly type: 'meeting';
  readonly index: number;
}
export type ModuleSlot = { readonly type: 'lobby' } | OfficeSlot | MeetingSlot;
export type ModuleMaterial = 'workshop' | 'moonlight' | 'copper';
export interface OfficeModule {
  readonly area: MapArea;
  readonly slot: ModuleSlot;
  readonly material: ModuleMaterial;
}
export interface ModuleMapDocument {
  readonly version: 2 | 3 | 4 | 5 | 6 | 7 | 8;
  readonly primaryLobbyId: string;
  readonly modules: readonly OfficeModule[];
}

function integer(value: unknown, min: number, max: number): number {
  if (!Number.isInteger(value) || Number(value) < min || Number(value) > max)
    throw new Error('Invalid Office module slot coordinate.');
  return Number(value);
}

function slot(value: unknown): ModuleSlot {
  const type = (value as { type?: unknown } | null)?.type;
  if (type === 'lobby') {
    exactRecord(value, ['type'], 'Lobby slot');
    return { type };
  }
  if (type === 'office') {
    const data = exactRecord(value, ['type', 'column', 'row'], 'office slot');
    return {
      type,
      column: integer(data.column, -2147483648, 2147483647),
      row: integer(data.row, -2147483648, 2147483647),
    };
  }
  if (type === 'meeting') {
    const data = exactRecord(value, ['type', 'index'], 'meeting slot');
    return { type, index: integer(data.index, 0, 4294967295) };
  }
  throw new Error('Invalid Office module slot.');
}

/** Clone the module source only. Geometry is derived, never accepted on the wire. */
export function decodeModuleMap(value: unknown): ModuleMapDocument {
  const data = exactRecord(value, ['version', 'primaryLobbyId', 'modules'], 'module map');
  if (
    (data.version !== 2 &&
      data.version !== 3 &&
      data.version !== 4 &&
      data.version !== 5 &&
      data.version !== 6 &&
      data.version !== 7 &&
      data.version !== 8) ||
    !Array.isArray(data.modules) ||
    data.modules.length > MAP_LIMITS.areas
  )
    throw new Error('Invalid Office module map.');
  const ids = new Set<string>();
  return {
    version: data.version,
    primaryLobbyId: canonicalUuid(data.primaryLobbyId),
    modules: data.modules.map((value) => {
      const data = exactRecord(value, ['area', 'slot', 'material'], 'module');
      if (
        data.material !== 'workshop' &&
        data.material !== 'moonlight' &&
        data.material !== 'copper'
      )
        throw new Error('Unknown Office material.');
      const area = decodeMapArea(data.area);
      if (ids.has(area.id)) throw new Error('Duplicate Office module area ID.');
      ids.add(area.id);
      return { area, slot: slot(data.slot), material: data.material };
    }),
  };
}
