/** Frozen resource field/query mapping. Names are release data, never a Colab-specific list. */
import { RefusalError } from './operations.js';
export interface SyncQueryMapping {
  readonly scopeFields: readonly string[];
  readonly streamField: string;
  readonly seqField: string;
  readonly order: 'asc';
}
export interface SyncMapping {
  readonly fields: readonly string[];
  readonly spaceField: string;
  readonly uidField: string | null;
  readonly query: SyncQueryMapping | null;
}
export const syncFieldName = (value: string): boolean => /^[A-Za-z][A-Za-z0-9]{0,63}$/.test(value);
function valid(condition: boolean): asserts condition {
  if (!condition) throw new RefusalError('REMOTE_INPUT_INVALID');
}
function row(value: unknown, keys: string[]): Record<string, unknown> {
  valid(typeof value === 'object' && value !== null && !Array.isArray(value));
  const r = value as Record<string, unknown>;
  valid(Object.keys(r).sort().join(',') === keys.sort().join(','));
  return r;
}
export function parseSyncMapping(value: unknown): SyncMapping {
  const r = row(value, ['fields', 'spaceField', 'uidField', 'query']);
  valid(
    Array.isArray(r.fields) &&
      r.fields.length > 0 &&
      r.fields.length <= 64 &&
      r.fields.every((f) => typeof f === 'string' && syncFieldName(f)),
  );
  const fields = new Set<string>(r.fields);
  valid(fields.size === r.fields.length);
  const declared = (v: unknown): v is string => typeof v === 'string' && fields.has(v);
  valid(
    declared(r.spaceField) &&
      (r.uidField === null || (declared(r.uidField) && r.uidField !== r.spaceField)),
  );
  let query: SyncQueryMapping | null = null;
  if (r.query !== null) {
    const q = row(r.query, ['scopeFields', 'streamField', 'seqField', 'order']);
    valid(
      Array.isArray(q.scopeFields) &&
        q.scopeFields.length > 0 &&
        q.scopeFields.length <= 8 &&
        q.scopeFields.every(declared),
    );
    const scope = new Set<string>(q.scopeFields);
    valid(scope.size === q.scopeFields.length);
    valid(
      declared(q.streamField) &&
        declared(q.seqField) &&
        q.streamField !== q.seqField &&
        q.order === 'asc' &&
        scope.has(r.spaceField) &&
        scope.has(q.streamField) &&
        !scope.has(q.seqField),
    );
    query = Object.freeze({
      scopeFields: Object.freeze([...q.scopeFields]),
      streamField: q.streamField,
      seqField: q.seqField,
      order: 'asc',
    });
  }
  return Object.freeze({
    fields: Object.freeze([...fields]),
    spaceField: r.spaceField,
    uidField: r.uidField as string | null,
    query,
  });
}
