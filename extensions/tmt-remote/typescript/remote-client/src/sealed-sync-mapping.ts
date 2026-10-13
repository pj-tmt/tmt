/** Frozen resource field/query mapping. Names are release data, never a Colab-specific list. */
import { RefusalError } from './operations.js';
export interface SyncQueryMapping {
  readonly scopeFields: readonly string[];
  readonly streamField: string;
  readonly seqField: string;
  readonly order: 'asc';
  readonly discovery?: SyncDiscoveryMapping;
}
export interface SyncDiscoveryMapping {
  readonly name: string;
  readonly scopeFields: readonly string[];
  readonly seqEquals: string | null;
}
export interface SyncMapping {
  readonly writeModes?: readonly ('create' | 'put' | 'delete')[];
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
  valid(typeof value === 'object' && value !== null);
  const r = row(value, [
    'fields',
    'spaceField',
    'uidField',
    'query',
    ...(Object.hasOwn(value, 'writeModes') ? ['writeModes'] : []),
  ]);
  if (r.writeModes !== undefined)
    valid(
      Array.isArray(r.writeModes) &&
        r.writeModes.length > 0 &&
        new Set(r.writeModes).size === r.writeModes.length &&
        r.writeModes.every((m) => ['create', 'put', 'delete'].includes(m)),
    );
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
    valid(typeof r.query === 'object' && r.query !== null);
    const q = row(r.query, [
      'scopeFields',
      'streamField',
      'seqField',
      'order',
      ...(Object.hasOwn(r.query, 'discovery') ? ['discovery'] : []),
    ]);
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
        q.streamField !== r.spaceField &&
        q.streamField !== r.uidField &&
        q.seqField !== r.uidField &&
        q.order === 'asc' &&
        scope.has(r.spaceField) &&
        scope.has(q.streamField) &&
        !scope.has(q.seqField),
    );
    let discovery: SyncDiscoveryMapping | undefined;
    if (q.discovery !== undefined) {
      const d = row(q.discovery, ['name', 'scopeFields', 'seqEquals']);
      valid(
        typeof d.name === 'string' &&
          syncFieldName(d.name) &&
          d.name !== 'stream' &&
          Array.isArray(d.scopeFields) &&
          d.scopeFields.every(declared) &&
          new Set(d.scopeFields).size === d.scopeFields.length,
      );
      valid(
        [...scope]
          .filter((n) => n !== q.streamField)
          .sort()
          .join(',') === [...d.scopeFields].sort().join(','),
      );
      valid(
        d.seqEquals === null ||
          (typeof d.seqEquals === 'string' &&
            Number.isSafeInteger(Number(d.seqEquals)) &&
            Number(d.seqEquals) >= 0 &&
            String(Number(d.seqEquals)) === d.seqEquals),
      );
      discovery = Object.freeze({
        name: d.name as string,
        scopeFields: Object.freeze([...d.scopeFields]),
        seqEquals: d.seqEquals as string | null,
      });
    }
    query = Object.freeze({
      scopeFields: Object.freeze([...q.scopeFields]),
      streamField: q.streamField,
      seqField: q.seqField,
      order: 'asc',
      ...(discovery ? { discovery } : {}),
    });
  }
  return Object.freeze({
    fields: Object.freeze([...fields]),
    ...(r.writeModes === undefined
      ? {}
      : { writeModes: Object.freeze([...(r.writeModes as ('create' | 'put' | 'delete')[])]) }),
    spaceField: r.spaceField,
    uidField: r.uidField as string | null,
    query,
  });
}
