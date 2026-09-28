import { snapshotHistory } from '../editor/snapshot-history.js';
import { decodeWorldDocument } from './world-contract.js';
import type { WorldDocument } from './world-contract.js';
import type { MapDocument } from './map-contract.js';

/** One gesture history for topology, occupancy, furniture and resource attachments. */
export const worldHistory = snapshotHistory<WorldDocument>(decodeWorldDocument);

/** Topology edits retain dependent objects, including ones the edit invalidates. */
export function updateWorldMap(world: WorldDocument, map: MapDocument): WorldDocument {
  if (world.map.version !== 1) throw new Error('Edit module slots, not their derived floor.');
  return decodeWorldDocument({ ...world, map });
}

/** Removes only a placement. It does not call a resource or identity API. */
export function removeWorldObject(world: WorldDocument, id: string): WorldDocument {
  return decodeWorldDocument({
    ...world,
    objects: world.objects.filter((object) => object.id !== id),
  });
}
