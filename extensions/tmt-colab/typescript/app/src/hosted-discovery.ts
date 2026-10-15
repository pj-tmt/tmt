import type {
  SealedDocument,
  SealedSyncPort,
  SyncCall,
  SyncDiscovery,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';

/** Limits come from the same exact release declaration used to bind the port. */
export interface HostedRunLimit {
  documents: number;
  bytes: number;
}
/** A finite discovery run exposes no partial page set or transport-owned logical cursor. */
export class HostedDiscovery {
  constructor(
    private readonly port: SealedSyncPort,
    private readonly limits: ReadonlyMap<string, HostedRunLimit>,
  ) {}
  async run(
    query: Omit<SyncDiscovery, 'continuation'>,
    call: SyncCall,
  ): Promise<readonly SealedDocument[]> {
    const selected = structuredClone(query);
    const limit = this.limits.get(selected.resource);
    if (!limit || !whole(limit.documents) || !whole(limit.bytes)) throw unavailable();
    const documents: SealedDocument[] = [];
    const ids = new Set<string>(),
      tokens = new Set<string>();
    let continuation: string | undefined,
      bytes = 0;
    // Each continued page must add a document. The final empty page is still a charged query.
    for (let pages = 0; pages <= limit.documents; pages++) {
      live(call);
      let response;
      try {
        response = await this.port.query(
          { shape: 'discovery', query: { ...selected, ...(continuation ? { continuation } : {}) } },
          call,
        );
      } catch (error) {
        live(call);
        if (
          continuation &&
          typeof error === 'object' &&
          error !== null &&
          'code' in error &&
          error.code === 'REMOTE_INPUT_INVALID'
        )
          throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'catch-up-required' };
        throw error;
      }
      live(call);
      if (response.shape !== 'discovery') throw unavailable();
      const page = response.page;
      if (page.documents.length > 256) throw unavailable();
      let pageBytes = 0;
      for (const document of page.documents) {
        if (document.target.resource !== selected.resource || ids.has(document.target.id))
          throw unavailable();
        ids.add(document.target.id);
        pageBytes += document.bytes.byteLength;
        bytes += document.bytes.byteLength;
        documents.push(document);
        if (documents.length > limit.documents || bytes > limit.bytes || pageBytes > 397312)
          throw unavailable();
      }
      const next = page.continuation;
      if (next === undefined) return documents;
      if (
        typeof next !== 'string' ||
        next.length === 0 ||
        next.length > 1024 ||
        tokens.has(next) ||
        page.documents.length === 0
      )
        throw unavailable();
      tokens.add(next);
      continuation = next;
    }
    throw unavailable();
  }
}

const unavailable = () => ({ code: 'REMOTE_STATE_UNAVAILABLE' as const });
function whole(value: number): boolean {
  return Number.isSafeInteger(value) && value > 0;
}
function live(call: SyncCall): void {
  if (call.signal.aborted) throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' };
  if (!Number.isFinite(call.deadlineAtMs) || Date.now() >= call.deadlineAtMs)
    throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'deadline' };
}
