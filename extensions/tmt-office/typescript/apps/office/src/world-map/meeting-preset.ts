import { catalogFurniture, footprint } from '../blocks/block-contract.js';
import { mapGeometry } from './map-source.js';
import {
  WORKSHOP_FURNITURE,
  MODULAR_WORKSTATION,
  MODULAR_RECEPTION_DIGEST,
  BUILTIN_CATALOG,
} from '../props/prop-contract.js';
import {
  DISCUSSION_EXTENSION,
  WHITEBOARD_EXTENSION,
  BROADCASTER_EXTENSION,
} from '../extensions/bundled-extensions.js';
import type { ExtensionDefinition, ResourceBinding } from '../extensions/extension-contract.js';
import { freeFloorRows, floorRectangleIntervals } from './free-floor.js';
import { decodeWorldDocument } from './world-contract.js';
import type { WorldDocument, WorldObject } from './world-contract.js';
import { suggestWallPlacement } from './world-object-placement.js';

export const MEETING_PRESET_SIZE = { width: 36, height: 32 };

/** An explicit additive draft recipe, not a native default or resource creation API. */
export function addMeetingPreset(
  world: WorldDocument,
  areaId: string,
  newId: () => string = () => crypto.randomUUID()
): WorldDocument {
  const map = mapGeometry(world.map);
  const area = map.areas.find((area) => area.id === areaId);
  if (area?.binding.type !== 'meeting') throw new Error('Choose a meeting area first.');
  const { width, height } = MEETING_PRESET_SIZE;
  const platform = world.map.version >= 6;
  const free = freeFloorRows(
    map.floor.filter((span) => span.areaId === areaId),
    [
      ...world.objects
        .filter((object) => object.surface.type === 'floor')
        .map((object) => ({
          x: object.placement.x,
          y: object.placement.y,
          ...footprint(object.placement),
        })),
      // Floor-mounted sets must leave both sides of every threshold clear.
      // Unified areas can have neighbors on any of their four sides.
      ...(platform
        ? map.doors.map((edge) =>
            edge.axis === 'horizontal'
              ? { x: edge.x, y: edge.y - 1, width: 1, height: 2 }
              : { x: edge.x - 1, y: edge.y, width: 2, height: 1 }
          )
        : []),
    ],
    width
  );
  const y = [...free.keys()]
    .sort((a, b) => a - b)
    .find((row) => floorRectangleIntervals(free, row, height, width).length > 0);
  if (y === undefined)
    throw new Error(
      'The meeting set needs a clear 36 × 32 rectangle in this area. Extend it or move existing objects first.'
    );
  const origin = { x: floorRectangleIntervals(free, y, height, width)[0]!.start, y };
  const objects: WorldObject[] = [];
  function resource(definition: ExtensionDefinition, binding: ResourceBinding) {
    const object: WorldObject = {
      id: newId(),
      kind: 'decoration',
      surface: { type: 'floor' },
      placement: {
        ...definition.appearance,
        x: origin.x + (platform ? objects.length * 12 : 0),
        y: origin.y + (platform ? 24 : 0),
        rotation: 0,
      },
      extension: { definition: definition.id, binding },
    };
    objects.push(
      platform
        ? object
        : suggestWallPlacement(
            { ...world, objects: [...world.objects, ...objects] },
            object,
            areaId
          )
    );
  }
  // One shared whiteboard reference per canonical meeting, including multiple
  // physical areas for the same room. No document exists until someone saves it.
  resource(WHITEBOARD_EXTENSION, { kind: 'whiteboard', documentId: area.binding.roomId });
  resource(DISCUSSION_EXTENSION, { kind: 'office-board', roomId: area.binding.roomId });
  resource(BROADCASTER_EXTENSION, { kind: 'office-broadcast' });
  const reception = BUILTIN_CATALOG.find((pack) => pack.digest === MODULAR_RECEPTION_DIGEST)!;
  for (const [pack, key, dx, dy, rotation] of [
    [reception, 'reception-table', 10, 0, 0],
    [MODULAR_WORKSTATION, 'workstation-chair', 2, 0, 1],
    [MODULAR_WORKSTATION, 'workstation-chair', 28, 0, 3],
    [MODULAR_WORKSTATION, 'workstation-chair', 14, 0, 2],
    [MODULAR_WORKSTATION, 'workstation-chair', 14, 12, 0],
    [WORKSHOP_FURNITURE, 'leafy-plant', platform ? 30 : 0, platform ? 16 : 26, 0],
  ] as const) {
    objects.push({
      id: newId(),
      kind: 'decoration',
      surface: { type: 'floor' },
      extension: null,
      placement: catalogFurniture(pack, key, origin.x + dx, origin.y + dy, rotation),
    });
  }
  return decodeWorldDocument({ ...world, objects: [...world.objects, ...objects] });
}
