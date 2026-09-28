import { exactRecord, uuidIdentifier } from '../contracts/record.js';
import { decodeWhiteboardScene } from './scene-contract.js';
import type { WhiteboardScene } from './scene-contract.js';

export interface WhiteboardDocument {
  id: string;
  revision: number;
  scene: WhiteboardScene;
  updatedAtMs: number;
}

export interface WhiteboardSave {
  expectedRevision: number;
  operationId: string;
  scene: WhiteboardScene;
}

export interface WhiteboardReceipt {
  documentId: string;
  operationId: string;
  revision: number;
  changed: boolean;
  updatedAtMs: number;
}

/** No subscription or implicit creation. The editor owns drafts and explicit saves. */
export interface WhiteboardPort {
  show(id: string, signal?: AbortSignal): Promise<WhiteboardDocument>;
  save(id: string, input: WhiteboardSave, signal?: AbortSignal): Promise<WhiteboardReceipt>;
}

export function whiteboardId(value: unknown): string {
  return value === 'lobby' ? value : uuidIdentifier(value);
}

export function whiteboardCounter(value: unknown, minimum: number): number {
  if (!Number.isSafeInteger(value) || (value as number) < minimum)
    throw new Error('Invalid whiteboard revision or timestamp.');
  return value as number;
}

export function decodeWhiteboardDocument(value: unknown): WhiteboardDocument {
  const record = exactRecord(
    value,
    ['id', 'revision', 'scene', 'updatedAtMs'],
    'whiteboard document'
  );
  const id = whiteboardId(record.id);
  const revision = whiteboardCounter(record.revision, 0);
  const updatedAtMs = whiteboardCounter(record.updatedAtMs, revision === 0 ? 0 : 1);
  const scene = decodeWhiteboardScene(record.scene);
  if (
    revision === 0 &&
    (updatedAtMs !== 0 || scene.background !== '#fff7e7' || scene.elements.length !== 0)
  )
    throw new Error('Invalid default whiteboard document.');
  return { id, revision, scene, updatedAtMs };
}

/** Capture an owned intent before awaiting transport; retries reuse this operation ID. */
export function decodeWhiteboardSave(value: unknown): WhiteboardSave {
  const record = exactRecord(
    value,
    ['expectedRevision', 'operationId', 'scene'],
    'whiteboard save'
  );
  return {
    expectedRevision: whiteboardCounter(record.expectedRevision, 0),
    operationId: uuidIdentifier(record.operationId),
    scene: decodeWhiteboardScene(record.scene),
  };
}

export function decodeWhiteboardReceipt(value: unknown): WhiteboardReceipt {
  const record = exactRecord(
    value,
    ['documentId', 'operationId', 'revision', 'changed', 'updatedAtMs'],
    'whiteboard receipt'
  );
  if (typeof record.changed !== 'boolean') throw new Error('Invalid whiteboard change receipt.');
  return {
    documentId: whiteboardId(record.documentId),
    operationId: uuidIdentifier(record.operationId),
    revision: whiteboardCounter(record.revision, 1),
    changed: record.changed,
    updatedAtMs: whiteboardCounter(record.updatedAtMs, 1),
  };
}
