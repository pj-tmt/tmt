import { exactKeys, requireValue, strictJson } from '@tmt/colab-client';
import type {
  SealedDocument,
  SealedSyncPort,
  SyncCall,
  SyncQuery,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import type { HostedRunLimit } from './hosted-discovery.js';

/** Finite stream read. These temporary page positions never advance Colab's verified head. */
export async function hostedQuery(
  port: SealedSyncPort,
  query: SyncQuery,
  limit: HostedRunLimit,
  call: SyncCall,
): Promise<readonly SealedDocument[]> {
  const selected = structuredClone(query),
    maximum = { ...limit },
    mapping = port.context.resources.find((resource) => resource.name === selected.resource)?.sync
      ?.query;
  if (!mapping || !whole(maximum.documents, 1) || !whole(maximum.bytes, 1)) throw unavailable();
  let after = selected.cursor?.seq ?? 0,
    bytes = 0;
  if (!whole(after, 0) || (selected.cursor && selected.cursor.stream !== selected.stream))
    throw unavailable();
  const documents: SealedDocument[] = [],
    ids = new Set<string>();
  // At least one new document per nonempty page, plus one final charged empty query.
  for (let pages = 0; pages <= maximum.documents; pages++) {
    live(call);
    const response = await port.query(
      {
        shape: 'stream',
        query: { ...selected, cursor: { stream: selected.stream, seq: after } },
      },
      call,
    );
    live(call);
    if (response.shape !== 'stream' || response.page.documents.length > 256) throw unavailable();
    let pageBytes = 0;
    for (const document of response.page.documents) {
      if (
        document.target.resource !== selected.resource ||
        ids.has(document.target.id) ||
        document.bytes.length > 393216
      )
        throw unavailable();
      const fields = strictJson(document.bytes, 393216);
      requireValue(typeof fields === 'object' && fields !== null && !Array.isArray(fields));
      const sequence = (fields as Record<string, unknown>)[mapping.seqField];
      exactKeys(sequence, ['integerValue']);
      if (
        typeof sequence.integerValue !== 'string' ||
        !/^[1-9][0-9]*$/.test(sequence.integerValue) ||
        !whole(Number(sequence.integerValue), 1)
      )
        throw unavailable();
      if (Number(sequence.integerValue) !== after + 1)
        throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'catch-up-required' };
      after++;
      ids.add(document.target.id);
      pageBytes += document.bytes.length;
      bytes += document.bytes.length;
      if (documents.length >= maximum.documents || bytes > maximum.bytes || pageBytes > 397312)
        throw unavailable();
      documents.push(document);
    }
    const cursor = response.page.cursor;
    if (
      (cursor && (cursor.stream !== selected.stream || cursor.seq !== after)) ||
      (response.page.documents.length > 0 && !cursor)
    )
      throw unavailable();
    if (response.page.documents.length === 0) return documents;
  }
  throw unavailable();
}
const unavailable = () => ({ code: 'REMOTE_STATE_UNAVAILABLE' as const });
function whole(value: number, minimum: number): boolean {
  return Number.isSafeInteger(value) && value >= minimum;
}
function live(call: SyncCall): void {
  if (call.signal.aborted) throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' };
  if (!Number.isFinite(call.deadlineAtMs) || Date.now() >= call.deadlineAtMs)
    throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'deadline' };
}
