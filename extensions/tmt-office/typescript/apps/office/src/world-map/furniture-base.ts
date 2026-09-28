import {
  BUILTIN_CATALOG,
  MODULAR_WORKSTATION_DIGEST,
  DIRECTIONAL_WORKSTATION_DIGEST,
  MODULAR_LOUNGE_DIGEST,
  DIRECTIONAL_LOUNGE_DIGEST,
  MODULAR_RECEPTION_DIGEST,
  DIRECTIONAL_RECEPTION_DIGEST,
  MODULAR_FACILITIES_DIGEST,
  DIRECTIONAL_FACILITIES_DIGEST,
  STUDY_DIGEST,
} from '../props/prop-contract.js';
import type { WorldObject } from './world-contract.js';
import { footprint } from '../blocks/block-contract.js';
import { floorObjectBounds } from './object-base.js';

// Authoring recipes, not art aliases or a read-time migration. The floor surface
// stores its physical base so support survives missing or removed artwork.
const BASE_SIZES: ReadonlyMap<
  string,
  Readonly<Record<string, readonly [number, number]>>
> = new Map<string, Readonly<Record<string, readonly [number, number]>>>([
  ...[MODULAR_WORKSTATION_DIGEST, DIRECTIONAL_WORKSTATION_DIGEST].map(
    (digest) =>
      [
        digest,
        {
          'workstation-desk': [14, 8],
          'workstation-chair': [4, 4],
          'workstation-terminal': [6, 3],
          'workstation-bookcase': [7, 3],
        },
      ] as const
  ),
  ...[MODULAR_LOUNGE_DIGEST, DIRECTIONAL_LOUNGE_DIGEST].map(
    (digest) =>
      [
        digest,
        {
          'lounge-sofa': [14, 6],
          'lounge-armchair': [6, 4],
          'lounge-table': [8, 6],
          'lounge-plant': [4, 2],
        },
      ] as const
  ),
  ...[MODULAR_RECEPTION_DIGEST, DIRECTIONAL_RECEPTION_DIGEST].map(
    (digest) =>
      [
        digest,
        {
          'reception-armchair': [8, 6],
          'reception-table': [14, 8],
        },
      ] as const
  ),
  ...[MODULAR_FACILITIES_DIGEST, DIRECTIONAL_FACILITIES_DIGEST].map(
    (digest) =>
      [
        digest,
        {
          'lobby-whiteboard': [14, 2],
          'lobby-discussion-board': [10, 2],
          'lobby-radio': [6, 3],
        },
      ] as const
  ),
  [STUDY_DIGEST, { 'oak-bookcase': [8, 3], 'reading-lamp': [3, 2], 'desktop-terminal': [4, 2] }],
]);

/** Adopt on explicit placement/move/turn only; preserve saved custom bases. */
export function withFurnitureBase(object: WorldObject, mapVersion: number): WorldObject {
  if (mapVersion < 6 || object.surface.type !== 'floor' || object.surface.base) return object;
  const [digest, key] = object.placement.prop.split('/');
  const size = digest && key && BASE_SIZES.get(digest)?.[key];
  const definition = BUILTIN_CATALOG.find((pack) => pack.digest === digest)?.pack.props.find(
    (prop) => prop.key === key
  );
  if (
    !size ||
    !definition ||
    definition.footprint.width !== object.placement.footprint.width ||
    definition.footprint.height !== object.placement.footprint.height
  )
    return object;
  const [width, height] = size;
  const candidate: WorldObject = {
    ...object,
    surface: {
      type: 'floor',
      base: {
        x: Math.floor((definition.footprint.width - width) / 2),
        y: definition.footprint.height - height,
        width,
        height,
      },
    },
  };
  // Keep the visible feet anchored when a retained placement first adopts a
  // base, including an object already turned before this authoring capability.
  const art = footprint(object.placement);
  const base = floorObjectBounds(candidate);
  const offsetX = base.x - object.placement.x;
  const offsetY = base.y - object.placement.y;
  return {
    ...candidate,
    placement: {
      ...object.placement,
      x: object.placement.x + Math.round(art.width / 2 - offsetX - base.width / 2),
      y: object.placement.y + art.height - offsetY - base.height,
    },
  };
}
