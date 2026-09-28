import { decodeMapDocument } from './map-contract.js';
import type { FloorSpan, MapDocument } from './map-contract.js';

/** Canonical row runs, independent of stroke subdivision. Never expands a bounding box. */
function canonicalFloor(floor: FloorSpan[]): FloorSpan[] {
  floor.sort((a, b) => a.y - b.y || a.start - b.start);
  const result: FloorSpan[] = [];
  for (const span of floor) {
    const previous = result.at(-1);
    if (previous && previous.y === span.y && previous.end > span.start)
      throw new Error('Overlapping Office floor.');
    if (
      previous &&
      previous.y === span.y &&
      previous.end === span.start &&
      previous.areaId === span.areaId
    )
      result[result.length - 1] = { ...previous, end: span.end };
    else result.push(span);
  }
  return result;
}

/** Canonical value order, not a reachability or occupancy admission decision. */
export function canonicalMapDraft(document: MapDocument): MapDocument {
  return decodeMapDocument({
    ...document,
    areas: [...document.areas].sort((left, right) =>
      left.id < right.id ? -1 : left.id > right.id ? 1 : 0
    ),
    floor: canonicalFloor([...document.floor]),
    doors: [...document.doors].sort(
      (left, right) =>
        left.x - right.x ||
        left.y - right.y ||
        (left.axis === right.axis ? 0 : left.axis === 'horizontal' ? -1 : 1)
    ),
  });
}

/** Detach the area only. Identities, room rosters, furniture and resource contents are not owned here. */
export function removeArea(
  document: MapDocument,
  id: string,
  replacementLobbyId?: string
): MapDocument {
  if (!document.areas.some((area) => area.id === id)) throw new Error('Unknown Office area.');
  let primaryLobbyId = document.primaryLobbyId;
  if (id === primaryLobbyId) {
    if (
      !document.areas.some(
        (area) => area.id !== id && area.id === replacementLobbyId && area.binding.type === 'lobby'
      )
    )
      throw new Error('Choose a replacement Lobby before removing the primary Lobby.');
    primaryLobbyId = replacementLobbyId!;
  }
  return decodeMapDocument({
    ...document,
    primaryLobbyId,
    areas: document.areas.filter((area) => area.id !== id),
    floor: canonicalFloor(
      document.floor.map((span) => (span.areaId === id ? { ...span, areaId: null } : span))
    ),
  });
}
