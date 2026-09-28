import { builtinFurniture } from '../../../extensions/tmt-office/typescript/apps/office/src/blocks/block-contract.js';
import type { WorldSnapshot } from '../../../extensions/tmt-office/typescript/apps/office/src/world-map/world-port.js';
import type { MapDocument } from '../../../extensions/tmt-office/typescript/apps/office/src/world-map/map-contract.js';
import type {
  WorldDocument,
  WorldObject,
} from '../../../extensions/tmt-office/typescript/apps/office/src/world-map/world-contract.js';
import lobby from '../../../extensions/tmt-office/contracts/lobby-preset-v1.json' with { type: 'json' };
import discussion from '../../../extensions/tmt-office/contracts/discussion-extension-v1.json' with { type: 'json' };
import whiteboard from '../../../extensions/tmt-office/contracts/whiteboard-extension-v1.json' with { type: 'json' };
import broadcaster from '../../../extensions/tmt-office/contracts/broadcaster-extension-v1.json' with { type: 'json' };
import discussionInstance from '../../../extensions/tmt-office/contracts/lobby-extension-v1.json' with { type: 'json' };
import whiteboardInstance from '../../../extensions/tmt-office/contracts/lobby-whiteboard-v1.json' with { type: 'json' };
import broadcasterInstance from '../../../extensions/tmt-office/contracts/lobby-broadcaster-v1.json' with { type: 'json' };
import { decodeExtensionAttachment } from '../../../extensions/tmt-office/typescript/apps/office/src/extensions/extension-contract.js';

/** Explicit retained-block composition. Tests of older topology must not inherit
 * the changing new-install preset or translate its mounted objects as floor art.
 */
export function legacyLobbyObjects(): WorldObject[] {
  const entries = [
    [discussion, discussionInstance],
    [whiteboard, whiteboardInstance],
    [broadcaster, broadcasterInstance],
  ] as const;
  const objects: WorldObject[] = entries.map(([definition, instance], index) => ({
    id: `30000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
    kind: 'decoration',
    surface: { type: 'floor' },
    placement: {
      ...definition.appearance,
      x: instance.x + 2,
      y: instance.y + 2,
      rotation: instance.rotation,
    },
    extension: decodeExtensionAttachment({
      definition: instance.definition,
      binding: instance.binding,
    }),
  }));
  for (const item of lobby.objects)
    objects.push({
      id: `30000000-0000-4000-8000-${String(objects.length + 1).padStart(12, '0')}`,
      kind: 'decoration',
      surface: { type: 'floor' },
      placement: { ...item, x: item.x + 38, y: item.y + 2 },
      extension: null,
    });
  return objects;
}

export const WORLD_LOBBY_ID = '10000000-0000-4000-8000-000000000001';
export function officeWorldFixture(): WorldSnapshot & {
  layout: WorldDocument & { map: MapDocument };
} {
  return {
    worldId: '10000000-0000-4000-8000-000000000009',
    revision: 1,
    legacyBasis: null,
    updatedAtMs: 1,
    changed: false,
    layout: {
      version: 1,
      map: {
        version: 1,
        primaryLobbyId: WORLD_LOBBY_ID,
        areas: [{ id: WORLD_LOBBY_ID, name: 'Lobby', binding: { type: 'lobby' } }],
        floor: Array.from({ length: 36 }, (_, y) => ({
          y,
          start: 0,
          end: 36,
          areaId: WORLD_LOBBY_ID,
        })),
        doors: [],
      },
      objects: [
        {
          id: '30000000-0000-4000-8000-000000000001',
          kind: 'decoration',
          placement: builtinFurniture('desk', 4, 4, 0),
          surface: { type: 'floor' },
          extension: null,
        },
      ],
    },
  };
}
