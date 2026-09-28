import { boundedText, canonicalUuid, exactRecord } from '../contracts/record.js';

/** Wire budgets, matching native map-v1 admission; these are not viewport dimensions. */
export const MAP_LIMITS = {
  documentBytes: 2 * 1024 * 1024,
  tiles: 262_144,
  spans: 16_384,
  areas: 256,
  doors: 4096,
  coordinate: 4096,
} as const;

export type AreaBinding =
  | { readonly type: 'lobby' }
  | { readonly type: 'personal'; readonly identityId: string | null }
  | { readonly type: 'meeting'; readonly roomId: string };
export interface MapArea {
  readonly id: string;
  readonly name: string;
  readonly binding: AreaBinding;
}
export interface FloorSpan {
  readonly y: number;
  readonly start: number;
  readonly end: number;
  readonly areaId: string | null;
}
export interface MapEdge {
  readonly x: number;
  readonly y: number;
  readonly axis: 'horizontal' | 'vertical';
}
export interface MapDocument {
  readonly version: 1;
  readonly primaryLobbyId: string;
  readonly areas: readonly MapArea[];
  readonly floor: readonly FloorSpan[];
  readonly doors: readonly MapEdge[];
}

function integer(value: unknown): number {
  if (!Number.isInteger(value) || (value as number) < -2147483648 || (value as number) > 2147483647)
    throw new Error('Invalid map integer.');
  return value as number;
}

function array<T>(value: unknown, limit: number, decode: (item: unknown) => T): T[] {
  if (!Array.isArray(value) || value.length > limit)
    throw new Error('Office map exceeds its geometry budget.');
  return value.map(decode);
}

function binding(value: unknown): AreaBinding {
  if (!value || typeof value !== 'object' || !('type' in value))
    throw new Error('Invalid area binding.');
  switch (value.type) {
    case 'lobby':
      exactRecord(value, ['type'], 'Lobby binding');
      return { type: 'lobby' };
    case 'personal': {
      const data = exactRecord(value, ['type', 'identityId'], 'personal binding');
      return {
        type: 'personal',
        identityId: data.identityId === null ? null : canonicalUuid(data.identityId),
      };
    }
    case 'meeting': {
      const data = exactRecord(value, ['type', 'roomId'], 'meeting binding');
      return { type: 'meeting', roomId: canonicalUuid(data.roomId) };
    }
    default:
      throw new Error('Invalid area binding.');
  }
}

export function decodeMapArea(value: unknown): MapArea {
  const area = exactRecord(value, ['id', 'name', 'binding'], 'area');
  if (
    !boundedText(area.name, 80) ||
    !/[^\p{White_Space}]/u.test(area.name) ||
    /[\u2028-\u202e\u2066-\u2069]/u.test(area.name)
  )
    throw new Error('Invalid Office area label.');
  return { id: canonicalUuid(area.id), name: area.name, binding: binding(area.binding) };
}

/**
 * Strict shape/resource-boundary decoding, not save authorization. In-progress
 * drafts may be disconnected or temporarily lack a Lobby. The native topology
 * owner checks reachability, occupancy and resource validity before committing.
 */
export function decodeMapDocument(value: unknown): MapDocument {
  const data = exactRecord(value, ['version', 'primaryLobbyId', 'areas', 'floor', 'doors'], 'map');
  if (data.version !== 1) throw new Error('Unsupported Office map version.');
  let tiles = 0;
  return {
    version: 1,
    primaryLobbyId: canonicalUuid(data.primaryLobbyId),
    areas: array(data.areas, MAP_LIMITS.areas, decodeMapArea),
    floor: array(data.floor, MAP_LIMITS.spans, (item) => {
      const span = exactRecord(item, ['y', 'start', 'end', 'areaId'], 'floor span');
      const y = integer(span.y);
      const start = integer(span.start);
      const end = integer(span.end);
      if (
        y < -MAP_LIMITS.coordinate ||
        y >= MAP_LIMITS.coordinate ||
        start < -MAP_LIMITS.coordinate ||
        end > MAP_LIMITS.coordinate ||
        start >= end
      )
        throw new Error('Invalid Office floor span.');
      tiles += end - start;
      if (tiles > MAP_LIMITS.tiles) throw new Error('Office map exceeds its tile budget.');
      return { y, start, end, areaId: span.areaId === null ? null : canonicalUuid(span.areaId) };
    }),
    doors: array(data.doors, MAP_LIMITS.doors, (item) => {
      const door = exactRecord(item, ['x', 'y', 'axis'], 'door');
      if (door.axis !== 'horizontal' && door.axis !== 'vertical')
        throw new Error('Invalid Office edge axis.');
      return { x: integer(door.x), y: integer(door.y), axis: door.axis };
    }),
  };
}
