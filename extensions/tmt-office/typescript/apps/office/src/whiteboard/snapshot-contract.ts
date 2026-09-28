import { exactRecord, uuidIdentifier } from '../contracts/record.js';
import { whiteboardCounter, whiteboardId } from './document-contract.js';
import { decodeWhiteboardScene, WHITEBOARD_LIMITS, whiteboardText } from './scene-contract.js';
import type { WhiteboardScene } from './scene-contract.js';

export const SNAPSHOT_PNG_LIMIT = 8 * 1024 * 1024;

export interface WhiteboardCapture {
  expectedRevision: number;
  operationId: string;
  selectedElementIds: string[];
  annotation: string;
}

export interface WhiteboardSnapshot {
  id: string;
  documentId: string;
  documentRevision: number;
  scene: WhiteboardScene;
  selectedElementIds: string[];
  annotation: string;
  createdAtMs: number;
}

export interface WhiteboardSnapshotPort {
  capture(
    documentId: string,
    input: WhiteboardCapture,
    signal?: AbortSignal
  ): Promise<WhiteboardSnapshot>;
  show(id: string, signal?: AbortSignal): Promise<WhiteboardSnapshot>;
  attachImage(id: string, image: Blob, signal?: AbortSignal): Promise<Blob>;
  image(id: string, signal?: AbortSignal): Promise<Blob>;
}

function selection(value: unknown): string[] {
  if (!Array.isArray(value) || value.length > WHITEBOARD_LIMITS.elements)
    throw new Error('Invalid whiteboard selection.');
  const ids = value.map(uuidIdentifier);
  if (new Set(ids).size !== ids.length) throw new Error('Duplicate whiteboard selection.');
  return ids.sort();
}

export function decodeWhiteboardCapture(value: unknown): WhiteboardCapture {
  const record = exactRecord(
    value,
    ['expectedRevision', 'operationId', 'selectedElementIds', 'annotation'],
    'whiteboard capture'
  );
  return {
    expectedRevision: whiteboardCounter(record.expectedRevision, 1),
    operationId: uuidIdentifier(record.operationId),
    selectedElementIds: selection(record.selectedElementIds),
    annotation: whiteboardText(record.annotation),
  };
}

export function decodeWhiteboardSnapshot(value: unknown): WhiteboardSnapshot {
  const record = exactRecord(
    value,
    [
      'id',
      'documentId',
      'documentRevision',
      'scene',
      'selectedElementIds',
      'annotation',
      'createdAtMs',
    ],
    'whiteboard snapshot'
  );
  const scene = decodeWhiteboardScene(record.scene);
  const selectedElementIds = selection(record.selectedElementIds);
  const present = new Set(scene.elements.map((element) => element.id));
  if (selectedElementIds.some((id) => !present.has(id)))
    throw new Error('Snapshot selection is missing from its scene.');
  return {
    id: uuidIdentifier(record.id),
    documentId: whiteboardId(record.documentId),
    documentRevision: whiteboardCounter(record.documentRevision, 1),
    scene,
    selectedElementIds,
    annotation: whiteboardText(record.annotation),
    createdAtMs: whiteboardCounter(record.createdAtMs, 1),
  };
}

export function checkSnapshotImage(image: Blob): Blob {
  if (image.type !== 'image/png' || image.size === 0 || image.size > SNAPSHOT_PNG_LIMIT)
    throw new Error('Snapshot image must be a PNG within 8 MiB.');
  return image;
}
