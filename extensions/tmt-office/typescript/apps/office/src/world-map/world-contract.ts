import { canonicalUuid, exactRecord } from '../contracts/record.js';
import { validPlacement, sameLayout } from '../blocks/block-contract.js';
import type { Furniture } from '../blocks/block-contract.js';
import { decodeExtensionAttachment } from '../extensions/extension-contract.js';
import type { ExtensionAttachment } from '../extensions/extension-contract.js';
import { decodeMapSource, canonicalMapSource } from './map-source.js';
import type { MapSource } from './map-source.js';
import { decodeFloorBase } from './object-base.js';
import type { FloorBase } from './object-base.js';

export const WORLD_LIMITS = {
  objects: 4096,
  documentBytes: 4 * 1024 * 1024,
  wallHeight: 16,
} as const;
export type ObjectSurface =
  | { readonly type: 'floor'; readonly base?: FloorBase }
  | {
      readonly type: 'wall';
      readonly axis: 'horizontal' | 'vertical';
      readonly face: 'positive' | 'negative';
      readonly elevation: number;
    };
export interface WorldObject {
  readonly id: string;
  readonly kind: 'decoration' | 'window' | 'wallLight';
  readonly placement: Furniture;
  readonly surface: ObjectSurface;
  readonly extension: ExtensionAttachment | null;
}
export interface WorldDocument {
  readonly version: 1;
  readonly map: MapSource;
  readonly objects: readonly WorldObject[];
}

/** Native ordering may coalesce brush runs; object paint order is never normalized. */
export function sameWorld(left: WorldDocument, right: WorldDocument): boolean {
  return (
    JSON.stringify(canonicalMapSource(left.map)) ===
      JSON.stringify(canonicalMapSource(right.map)) &&
    sameLayout(
      left.objects.map((object) => object.placement),
      right.objects.map((object) => object.placement)
    ) &&
    left.objects.every((object, index) => {
      const other = right.objects[index];
      return (
        object.id === other?.id &&
        object.kind === other.kind &&
        JSON.stringify(object.surface) === JSON.stringify(other.surface) &&
        JSON.stringify(object.extension) === JSON.stringify(other.extension)
      );
    })
  );
}

function surface(value: unknown, placement: Furniture): ObjectSurface {
  const type = (value as { type?: unknown } | null)?.type;
  if (type === 'floor') {
    const hasBase = Object.hasOwn(value as object, 'base');
    const data = exactRecord(value, hasBase ? ['type', 'base'] : ['type'], 'floor surface');
    return hasBase ? { type, base: decodeFloorBase(data.base, placement) } : { type };
  }
  const data = exactRecord(value, ['type', 'axis', 'face', 'elevation'], 'wall surface');
  if (
    type !== 'wall' ||
    (data.axis !== 'horizontal' && data.axis !== 'vertical') ||
    (data.face !== 'positive' && data.face !== 'negative') ||
    !Number.isInteger(data.elevation) ||
    Number(data.elevation) < 0 ||
    Number(data.elevation) > 255
  )
    throw new Error('Invalid wall surface.');
  return { type, axis: data.axis, face: data.face, elevation: Number(data.elevation) };
}

/** Own the entire draft; geometry may temporarily be invalid until native Save admission. */
export function decodeWorldDocument(value: unknown): WorldDocument {
  const data = exactRecord(value, ['version', 'map', 'objects'], 'Office world');
  if (
    data.version !== 1 ||
    !Array.isArray(data.objects) ||
    data.objects.length > WORLD_LIMITS.objects
  )
    throw new Error('Invalid Office world envelope.');
  const ids = new Set<string>();
  const result: WorldDocument = {
    version: 1,
    map: decodeMapSource(data.map),
    objects: data.objects.map((value): WorldObject => {
      const object = exactRecord(
        value,
        ['id', 'kind', 'placement', 'surface', 'extension'],
        'world object'
      );
      const id = canonicalUuid(object.id);
      if (ids.has(id)) throw new Error('Duplicate Office object ID.');
      ids.add(id);
      if (object.kind !== 'decoration' && object.kind !== 'window' && object.kind !== 'wallLight')
        throw new Error('Invalid Office object kind.');
      if (!validPlacement(object.placement)) throw new Error('Invalid Office object appearance.');
      const placement = object.placement;
      return {
        id,
        kind: object.kind,
        placement: {
          ...placement,
          footprint: { ...placement.footprint },
          ...(placement.customization ? { customization: { ...placement.customization } } : {}),
        },
        surface: surface(object.surface, placement),
        extension: object.extension === null ? null : decodeExtensionAttachment(object.extension),
      };
    }),
  };
  if (new TextEncoder().encode(JSON.stringify(result)).length > WORLD_LIMITS.documentBytes)
    throw new Error('Office world exceeds its document budget.');
  return result;
}
