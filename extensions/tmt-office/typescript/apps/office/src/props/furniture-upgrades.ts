import {
  MODULAR_WORKSTATION_DIGEST,
  MODULAR_LOUNGE_DIGEST,
  MODULAR_RECEPTION_DIGEST,
  MODULAR_FACILITIES_DIGEST,
  DIRECTIONAL_WORKSTATION_DIGEST,
  DIRECTIONAL_LOUNGE_DIGEST,
  DIRECTIONAL_RECEPTION_DIGEST,
  DIRECTIONAL_FACILITIES_DIGEST,
  resolvePlacedProp,
} from './prop-contract.js';
import type { CatalogPack } from './prop-contract.js';
import type { Furniture } from '../blocks/block-contract.js';

/** Authoring successors, never a renderer alias or a saved-world migration.
 * Both immutable packs remain resolvable. Rotation commits the replacement with
 * the placement in one history entry; Undo restores the exact original bytes.
 */
const DIRECTIONAL_SUCCESSORS: Readonly<Record<string, string>> = {
  [MODULAR_WORKSTATION_DIGEST]: DIRECTIONAL_WORKSTATION_DIGEST,
  [MODULAR_LOUNGE_DIGEST]: DIRECTIONAL_LOUNGE_DIGEST,
  [MODULAR_RECEPTION_DIGEST]: DIRECTIONAL_RECEPTION_DIGEST,
  [MODULAR_FACILITIES_DIGEST]: DIRECTIONAL_FACILITIES_DIGEST,
};

export function directionalFurniture(item: Furniture, catalog: readonly CatalogPack[]): Furniture {
  const [digest, key] = item.prop.split('/');
  const successor = digest && DIRECTIONAL_SUCCESSORS[digest];
  if (!successor || !key) return item;
  const replacement = { ...item, prop: `${successor}/${key}` };
  return resolvePlacedProp(catalog, item) && resolvePlacedProp(catalog, replacement)
    ? replacement
    : item;
}
