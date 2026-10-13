/** Finite stream/discovery REST pages. Only callers own logical delivery positions. */
import {
  validateSyncQuery,
  validateSyncDiscovery,
  validateSyncFields,
  syncDocumentPath,
  type SyncQueryRequest,
  type SyncQueryPage,
  type SyncCall,
  type SyncContext,
  type SyncResource,
  type SealedDocument,
} from './sealed-sync.js';
import { base64url, base64urlBytes } from './canonical-bytes.js';
import { syncCallLive, syncWait, type syncHttp } from './firebase-sync-http.js';
const unavailable = () => ({ code: 'REMOTE_STATE_UNAVAILABLE' as const });
const invalid = () => ({ code: 'REMOTE_INPUT_INVALID' as const });
export function createSyncQuery(
  context: SyncContext,
  http: ReturnType<typeof syncHttp>,
  resource: (name: string) => SyncResource,
  document: (value: unknown, resource: SyncResource) => Promise<SealedDocument>,
  status: (code: number) => never,
  live: () => void,
) {
  const base = `projects/${context.config.projectId}/databases/(default)/documents/`;
  let continuationKey: Promise<CryptoKey> | undefined;
  const key = () =>
    (continuationKey ??= crypto.subtle.generateKey({ name: 'HMAC', hash: 'SHA-256' }, false, [
      'sign',
      'verify',
    ]));
  const encodeContinuation = async (binding: string, last: string, deadline: number) => {
    const bytes = new TextEncoder().encode(JSON.stringify({ binding, last, deadline }));
    const signature = new Uint8Array(await crypto.subtle.sign('HMAC', await key(), bytes));
    const token = `${base64url(bytes)}.${base64url(signature)}`;
    if (token.length > 1024) throw unavailable();
    return token;
  };
  const decodeContinuation = async (token: string, binding: string): Promise<string> => {
    try {
      const parts = token.split('.');
      if (parts.length !== 2) throw invalid();
      const size = Math.floor((parts[0]!.length * 6) / 8);
      const bytes = base64urlBytes(parts[0]!, size);
      if (
        !(await crypto.subtle.verify(
          'HMAC',
          await key(),
          new Uint8Array(base64urlBytes(parts[1]!, 32)),
          new Uint8Array(bytes),
        ))
      )
        throw invalid();
      const row = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)) as Record<
        string,
        unknown
      >;
      if (
        row.binding !== binding ||
        typeof row.deadline !== 'number' ||
        Date.now() >= row.deadline ||
        typeof row.last !== 'string'
      )
        throw invalid();
      live();
      return row.last;
    } catch {
      throw invalid();
    }
  };
  return async (request: SyncQueryRequest, call: SyncCall): Promise<SyncQueryPage> => {
    // Copy declaration-bound inputs before any token or budget await.
    const q = structuredClone(request) as SyncQueryRequest;
    const ordinary = q.shape === 'stream';
    if (ordinary) validateSyncQuery(context, q.query);
    else validateSyncDiscovery(context, q.query);
    const r = resource(q.query.resource);
    const mapping = r.sync!.query!;
    // Reserve room for provider names/timestamps and JSON envelopes inside the one-frame cap.
    const limit = Math.max(1, Math.min(256, Math.floor(397312 / (r.maxObjectBytes + 4096))));
    const filters = Object.entries(q.query.scope).map(([fieldPath, value]) => ({
      fieldFilter: { field: { fieldPath }, op: 'EQUAL', value },
    }));
    if (ordinary) {
      if (q.query.cursor)
        filters.push({
          fieldFilter: {
            field: { fieldPath: mapping.seqField },
            op: 'GREATER_THAN',
            value: { integerValue: String(q.query.cursor.seq) },
          },
        });
    } else {
      if (mapping.discovery!.seqEquals !== null)
        filters.push({
          fieldFilter: {
            field: { fieldPath: mapping.seqField },
            op: 'EQUAL',
            value: { integerValue: mapping.discovery!.seqEquals },
          },
        });
    }
    const segments = `x/${context.extension}/${r.path}`.split('/');
    const collectionId = segments.pop()!;
    const parent = segments.join('/');
    const structuredQuery = {
      from: [{ collectionId }],
      where: { compositeFilter: { op: 'AND', filters } },
      orderBy: [
        {
          field: { fieldPath: ordinary ? mapping.seqField : '__name__' },
          direction: 'ASCENDING',
        },
      ],
      limit,
    };
    const bound = call;
    const binding = JSON.stringify([
      context.config.projectId,
      context.declarationDigest,
      context.space,
      q.query.resource,
      q.shape === 'discovery' ? q.query.shape : '',
      Object.entries(q.query.scope).sort(([a], [b]) => (a < b ? -1 : 1)),
    ]);
    const queryBody: typeof structuredQuery & {
      startAt?: { values: { referenceValue: string }[]; before: boolean };
    } = structuredQuery;
    if (q.shape === 'discovery' && q.query.continuation) {
      const last = await syncWait(bound, () => decodeContinuation(q.query.continuation!, binding));
      if (
        !last.startsWith(`${base}${parent}/${collectionId}/`) ||
        !/^[A-Za-z0-9_+-]{1,256}$/.test(last.slice(`${base}${parent}/${collectionId}/`.length))
      )
        throw invalid();
      live();
      syncCallLive(bound);
      queryBody.startAt = { values: [{ referenceValue: last }], before: false };
    }
    const response = await http(
      'query',
      parent,
      JSON.stringify({ structuredQuery: queryBody }),
      bound,
      limit,
    );
    if (response.kind === 'unknown' || response.status !== 200) {
      if (response.kind !== 'unknown') status(response.status);
      throw unavailable();
    }
    if (!Array.isArray(response.value) || response.value.length > limit + 1) throw unavailable();
    const documents: SealedDocument[] = [];
    for (const row of response.value) {
      if (
        typeof row !== 'object' ||
        row === null ||
        Object.keys(row).some(
          (key) => !['document', 'readTime', 'skippedResults', 'transaction', 'done'].includes(key),
        )
      )
        throw unavailable();
      if ('document' in row) documents.push(await document(row.document, r));
    }
    if (documents.length > limit) throw unavailable();
    for (const d of documents) {
      const fields = validateSyncFields(d.bytes);
      if (
        !Object.entries(q.query.scope).every(
          ([key, value]) => JSON.stringify(fields[key]) === JSON.stringify(value),
        )
      )
        throw unavailable();
    }
    live();
    syncCallLive(bound);
    if (ordinary) {
      let position = q.query.cursor?.seq ?? 0;
      for (const d of documents) {
        const fields = validateSyncFields(d.bytes);
        const sequence = Number(
          (fields[mapping.seqField] as { integerValue: string }).integerValue,
        );
        if (sequence !== position + 1)
          throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'catch-up-required' };
        position = sequence;
      }
      return {
        shape: 'stream',
        page: {
          documents,
          ...(documents.length ? { cursor: { stream: q.query.stream, seq: position } } : {}),
        },
      };
    }
    let continuation: string | undefined;
    if (documents.length === limit) {
      const last = `${base}${syncDocumentPath(context, documents.at(-1)!.target)}`;
      continuation = await syncWait(bound, () =>
        encodeContinuation(binding, last, bound.deadlineAtMs),
      );
      live();
      syncCallLive(bound);
    }
    return {
      shape: 'discovery',
      page: { documents, ...(continuation ? { continuation } : {}) },
    };
  };
}
