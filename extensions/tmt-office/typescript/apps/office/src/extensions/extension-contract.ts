import { boundedText, exactRecord, canonicalUuid } from '../contracts/record.js';
import { BLOCK_SIZE, validFurniture } from '../blocks/block-contract.js';
import type { Furniture } from '../blocks/block-contract.js';
import { indexedArtKey } from '../rendering/indexed-art-contract.js';
import { whiteboardId } from '../whiteboard/document-contract.js';

import { externalLink } from './external-link.js';

export type WorldCapability =
  | 'discussion.open'
  | 'whiteboard.open'
  | 'notebook.open'
  | 'broadcast.open'
  | 'link.open';
export type ResourceBinding =
  | { kind: 'office-board'; roomId?: string }
  | { kind: 'whiteboard'; documentId: string }
  | { kind: 'notebook'; identityId: string }
  | { kind: 'office-broadcast' }
  | { kind: 'external-link'; url: string };
export interface ExtensionAttachment {
  definition: string;
  binding: ResourceBinding;
}

export function decodeResourceBinding(value: unknown): ResourceBinding {
  const kind = (value as { kind?: unknown } | null)?.kind;
  const binding = exactRecord(
    value,
    kind === 'notebook'
      ? ['kind', 'identityId']
      : kind === 'whiteboard'
        ? ['kind', 'documentId']
        : kind === 'external-link'
          ? ['kind', 'url']
          : kind === 'office-board' && Object.hasOwn(value as object, 'roomId')
            ? ['kind', 'roomId']
            : ['kind'],
    'extension binding'
  );
  switch (kind) {
    case 'notebook':
      return { kind, identityId: canonicalUuid(binding.identityId) };
    case 'external-link':
      externalLink(binding.url);
      return { kind, url: binding.url as string };
    case 'whiteboard':
      return { kind, documentId: whiteboardId(binding.documentId) };
    case 'office-board':
      return binding.roomId === undefined
        ? { kind }
        : { kind, roomId: canonicalUuid(binding.roomId) };
    case 'office-broadcast':
      return { kind };
    default:
      throw new Error('Invalid extension resource kind.');
  }
}

export function decodeExtensionAttachment(value: unknown): ExtensionAttachment {
  const input = exactRecord(value, ['definition', 'binding'], 'extension attachment');
  if (!indexedArtKey(input.definition)) throw new Error('Invalid extension definition ID.');
  return { definition: input.definition, binding: decodeResourceBinding(input.binding) };
}
export interface ExtensionDefinition {
  formatVersion: 1;
  worldApiVersion: 1;
  id: string;
  label: string;
  appearance: Pick<Furniture, 'prop' | 'footprint'>;
  action: {
    id: 'open';
    label: string;
    capability: WorldCapability;
    resourceKind: ResourceBinding['kind'];
  };
}
export interface ExtensionInstance {
  formatVersion: 1;
  id: string;
  definition: string;
  space: 'commons';
  x: number;
  y: number;
  rotation: number;
  binding: ResourceBinding;
}

/** Definitions describe known capabilities; admission never grants their use. */
export function decodeExtensionDefinition(value: unknown): ExtensionDefinition {
  const input = exactRecord(
    value,
    ['formatVersion', 'worldApiVersion', 'id', 'label', 'appearance', 'action'],
    'extension definition'
  );
  const appearance = exactRecord(input.appearance, ['prop', 'footprint'], 'extension appearance');
  const action = exactRecord(
    input.action,
    ['id', 'label', 'capability', 'resourceKind'],
    'extension action'
  );
  if (
    input.formatVersion !== 1 ||
    input.worldApiVersion !== 1 ||
    !indexedArtKey(input.id) ||
    !boundedText(input.label, 80) ||
    !boundedText(action.label, 80) ||
    action.id !== 'open' ||
    !(
      (action.capability === 'discussion.open' && action.resourceKind === 'office-board') ||
      (action.capability === 'whiteboard.open' && action.resourceKind === 'whiteboard') ||
      (action.capability === 'notebook.open' && action.resourceKind === 'notebook') ||
      (action.capability === 'broadcast.open' && action.resourceKind === 'office-broadcast') ||
      (action.capability === 'link.open' && action.resourceKind === 'external-link')
    ) ||
    !validFurniture({ ...appearance, x: 0, y: 0, rotation: 0 })
  )
    throw new Error('Invalid extension definition values.');
  return input as unknown as ExtensionDefinition;
}

export function decodeExtensionInstance(value: unknown): ExtensionInstance {
  const input = exactRecord(
    value,
    ['formatVersion', 'id', 'definition', 'space', 'x', 'y', 'rotation', 'binding'],
    'extension instance'
  );
  decodeResourceBinding(input.binding);
  if (
    input.formatVersion !== 1 ||
    !indexedArtKey(input.id) ||
    !indexedArtKey(input.definition) ||
    input.space !== 'commons' ||
    !Number.isInteger(input.x) ||
    !Number.isInteger(input.y) ||
    !Number.isInteger(input.rotation) ||
    Number(input.x) < 0 ||
    Number(input.x) >= BLOCK_SIZE ||
    Number(input.y) < 0 ||
    Number(input.y) >= BLOCK_SIZE ||
    Number(input.rotation) < 0 ||
    Number(input.rotation) > 3
  )
    throw new Error('Invalid extension instance values.');
  return input as unknown as ExtensionInstance;
}
