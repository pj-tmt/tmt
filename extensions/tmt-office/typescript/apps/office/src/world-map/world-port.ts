import { boundedText, canonicalUuid, exactRecord } from '../contracts/record.js';
import { decodeWorldDocument, WORLD_LIMITS } from './world-contract.js';
import type { WorldDocument } from './world-contract.js';

export const WORLD_ENVELOPE_BYTES = WORLD_LIMITS.documentBytes + 512;
export interface WorldSnapshot {
  readonly worldId: string | null;
  readonly revision: number;
  readonly legacyBasis: string | null;
  readonly layout: WorldDocument;
  readonly updatedAtMs: number;
  readonly changed: boolean;
}
export interface WorldWrite {
  readonly expectedRevision: number;
  readonly legacyBasis: string | null;
  readonly layout: WorldDocument;
}
export interface WorldPort {
  show(signal?: AbortSignal): Promise<WorldSnapshot>;
  save(input: WorldWrite, signal?: AbortSignal): Promise<WorldSnapshot>;
}

export class WorldConflict extends Error {
  constructor() {
    super('Saved layout changed. Nothing was written. Your draft is kept.');
  }
}

export interface WorldPlacementIssue {
  readonly objectId: string | null;
  readonly reason: string;
}
export class WorldValidationError extends Error {
  constructor(
    message: string,
    readonly issues: readonly WorldPlacementIssue[]
  ) {
    super(message);
  }
}
export function decodeWorldValidationError(value: unknown): WorldValidationError {
  const data = exactRecord(value, ['error', 'message', 'issues'], 'Office validation result');
  if (
    data.error !== 'WORLD_INVALID' ||
    !boundedText(data.message, 256) ||
    !Array.isArray(data.issues) ||
    data.issues.length > WORLD_LIMITS.objects
  )
    throw new Error('Invalid Office validation result.');
  return new WorldValidationError(
    data.message,
    data.issues.map((value): WorldPlacementIssue => {
      const issue = exactRecord(value, ['objectId', 'reason'], 'placement issue');
      if (!boundedText(issue.reason, 64)) throw new Error('Invalid placement reason.');
      return {
        objectId: issue.objectId === null ? null : canonicalUuid(issue.objectId),
        reason: issue.reason,
      };
    })
  );
}

function counter(value: unknown): number {
  if (!Number.isSafeInteger(value) || Number(value) < 0)
    throw new Error('Invalid Office world counter.');
  return Number(value);
}
function basis(value: unknown, revision: number): string | null {
  if (revision > 0 && value === null) return null;
  if (revision === 0 && typeof value === 'string' && /^[0-9a-f]{64}$/.test(value)) return value;
  throw new Error('Invalid Office migration basis.');
}
export function decodeWorldSnapshot(value: unknown): WorldSnapshot {
  const data = exactRecord(
    value,
    ['worldId', 'revision', 'legacyBasis', 'layout', 'updatedAtMs', 'changed'],
    'Office world snapshot'
  );
  const revision = counter(data.revision);
  const updatedAtMs = counter(data.updatedAtMs);
  const worldId = data.worldId === null ? null : canonicalUuid(data.worldId);
  if (
    typeof data.changed !== 'boolean' ||
    (revision === 0 ? updatedAtMs !== 0 || data.changed : updatedAtMs === 0 || worldId === null)
  )
    throw new Error('Inconsistent Office world snapshot.');
  return {
    worldId,
    revision,
    legacyBasis: basis(data.legacyBasis, revision),
    layout: decodeWorldDocument(data.layout),
    updatedAtMs,
    changed: data.changed,
  };
}
export function decodeWorldWrite(value: unknown): WorldWrite {
  const data = exactRecord(
    value,
    ['expectedRevision', 'legacyBasis', 'layout'],
    'Office world save'
  );
  const expectedRevision = counter(data.expectedRevision);
  const result = {
    expectedRevision,
    legacyBasis: basis(data.legacyBasis, expectedRevision),
    layout: decodeWorldDocument(data.layout),
  };
  if (new TextEncoder().encode(JSON.stringify(result)).length > WORLD_ENVELOPE_BYTES)
    throw new Error('Office world save exceeds its byte budget.');
  return result;
}
