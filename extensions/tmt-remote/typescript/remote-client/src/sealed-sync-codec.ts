/** Restricted REST fields bytes; provider encoding is not an extra application envelope. */
import type { SealedDocument, SyncContext, SyncQuery } from './sealed-sync.js';
import { syncFieldName } from './sealed-sync-mapping.js';
import { RefusalError } from './operations.js';
function valid(condition: boolean): asserts condition {
  if (!condition) throw new RefusalError('REMOTE_INPUT_INVALID');
}
function timestamp(s: string): boolean {
  if (
    !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{3}(?:\d{3})?)?Z$/.test(s) ||
    s.startsWith('0000') ||
    s.endsWith('000Z')
  )
    return false;
  const base = s.slice(0, 19);
  const date = new Date(`${base}Z`);
  return Number.isFinite(date.valueOf()) && date.toISOString().slice(0, 19) === base;
}
function field(value: unknown): boolean {
  if (
    typeof value !== 'object' ||
    value === null ||
    Array.isArray(value) ||
    Object.keys(value).length !== 1
  )
    return false;
  const [kind, v] = Object.entries(value)[0]!;
  switch (kind) {
    case 'stringValue':
      return (
        typeof v === 'string' &&
        ![...v].some((c) => {
          const n = c.codePointAt(0)!;
          return n >= 0xd800 && n <= 0xdfff;
        })
      );
    case 'integerValue':
      return typeof v === 'string' && Number.isSafeInteger(Number(v)) && String(Number(v)) === v;
    case 'booleanValue':
      return typeof v === 'boolean';
    case 'nullValue':
      return v === null;
    case 'timestampValue':
      return typeof v === 'string' && timestamp(v);
    case 'bytesValue':
      try {
        return typeof v === 'string' && btoa(atob(v)) === v;
      } catch {
        return false;
      }
    default:
      return false;
  }
}
function canonical(value: unknown): string {
  if (typeof value !== 'object' || value === null || Array.isArray(value))
    return JSON.stringify(value);
  return `{${Object.entries(value)
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([key, v]) => `${JSON.stringify(key)}:${canonical(v)}`)
    .join(',')}}`;
}
export function validateSyncFields(bytes: Uint8Array): Readonly<Record<string, unknown>> {
  try {
    const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    const row: unknown = JSON.parse(text);
    valid(
      typeof row === 'object' && row !== null && !Array.isArray(row) && canonical(row) === text,
    );
    const fields = row as Record<string, unknown>;
    valid(
      Object.keys(fields).length <= 64 &&
        Object.entries(fields).every(([key, v]) => syncFieldName(key) && field(v)),
    );
    return fields;
  } catch {
    throw new RefusalError('REMOTE_INPUT_INVALID');
  }
}
function textIs(value: unknown, expected: string): boolean {
  return (
    typeof value === 'object' &&
    value !== null &&
    Object.keys(value).length === 1 &&
    (value as Record<string, unknown>).stringValue === expected
  );
}
export async function validateSyncDocument(
  context: SyncContext,
  document: SealedDocument,
  access: 'read' | 'publish',
): Promise<void> {
  const resource = context.resources.find((r) => r.name === document.target.resource);
  valid(
    resource !== undefined &&
      /^[A-Za-z0-9_+-]{1,256}$/.test(document.target.id) &&
      document.bytes.length <= resource.maxObjectBytes,
  );
  const mapping = resource.sync;
  if (!mapping) throw new RefusalError('REMOTE_STATE_UNAVAILABLE');
  const bytes = new Uint8Array(document.bytes);
  const expectedDigest = document.digest;
  const row = validateSyncFields(bytes);
  valid(
    Object.keys(row).length === mapping.fields.length &&
      mapping.fields.every((key) => Object.hasOwn(row, key)),
  );
  if (mapping.query) {
    const stream = row[mapping.query.streamField] as Record<string, unknown>;
    const sequence = row[mapping.query.seqField] as Record<string, unknown>;
    valid(
      typeof stream.stringValue === 'string' &&
        /^[A-Za-z0-9_+-]{1,128}$/.test(stream.stringValue) &&
        typeof sequence.integerValue === 'string' &&
        Number(sequence.integerValue) > 0,
    );
  }
  const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), (b) =>
    b.toString(16).padStart(2, '0'),
  ).join('');
  valid(digest === expectedDigest);
  if (
    !textIs(row[mapping.spaceField], context.space) ||
    (access === 'publish' &&
      mapping.uidField !== null &&
      !textIs(row[mapping.uidField], context.uid))
  )
    throw new RefusalError('REMOTE_SCOPE_DENIED');
}
export function validateSyncQuery(context: SyncContext, query: SyncQuery): void {
  const mapping = context.resources.find((r) => r.name === query.resource)?.sync;
  const spec = mapping?.query;
  if (!mapping || !spec) throw new RefusalError('REMOTE_STATE_UNAVAILABLE');
  valid(
    Object.keys(query.scope).length === spec.scopeFields.length &&
      spec.scopeFields.every((key) => Object.hasOwn(query.scope, key) && field(query.scope[key])) &&
      /^[A-Za-z0-9_+-]{1,128}$/.test(query.stream),
  );
  if (
    !textIs(query.scope[mapping.spaceField], context.space) ||
    !textIs(query.scope[spec.streamField], query.stream)
  )
    throw new RefusalError('REMOTE_SCOPE_DENIED');
  valid(
    !query.cursor ||
      (query.cursor.stream === query.stream &&
        Number.isSafeInteger(query.cursor.seq) &&
        query.cursor.seq >= 0),
  );
}
