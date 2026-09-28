import { decodeMapDocument } from './map-contract.js';
import type { MapDocument } from './map-contract.js';
import { canonicalMapDraft } from './map-draft.js';
import { decodeModuleMap } from './module-contract.js';
import type { ModuleMapDocument } from './module-contract.js';
import { projectModules } from './module-geometry.js';

export type MapSource = MapDocument | ModuleMapDocument;
export function decodeMapSource(value: unknown): MapSource {
  const version = (value as { version?: unknown } | null)?.version;
  const source =
    version === 2 ||
    version === 3 ||
    version === 4 ||
    version === 5 ||
    version === 6 ||
    version === 7 ||
    version === 8
      ? decodeModuleMap(value)
      : decodeMapDocument(value);
  // Reject unrenderable module geometry at the draft/transport boundary, where
  // callers can keep the previous world and show an error instead of crashing.
  // Native Save still owns reachability, placements and resource admission.
  mapGeometry(source);
  return source;
}

// Immutable world drafts are the keys. This cache has no write API or persistence.
const projections = new WeakMap<ModuleMapDocument, MapDocument>();
export function mapGeometry(source: MapSource): MapDocument {
  if (source.version === 1) return source;
  let projection = projections.get(source);
  if (!projection) {
    projection = projectModules(source);
    projections.set(source, projection);
  }
  return projection;
}

export function canonicalMapSource(source: MapSource): MapSource {
  return source.version === 1
    ? canonicalMapDraft(source)
    : {
        ...source,
        modules: [...source.modules].sort((a, b) =>
          a.area.id < b.area.id ? -1 : a.area.id > b.area.id ? 1 : 0
        ),
      };
}
