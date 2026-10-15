/** Fixed-project REST attempts. This module never retries a possibly applied mutation. */
import type { SyncCall, SyncContext, SyncFailure } from './sealed-sync.js';
import { WRITE_LOOKUPS } from './budget.js';

export interface SyncAdmissionCost {
  readonly operation: 'read' | 'query' | 'commit' | 'listen';
  /** Upper bound for a REST page; null means the unbounded matching listener tail below. */
  readonly reads: number | null;
  readonly writes: 0 | 1;
  readonly rulesLookupsPerDocument: 3 | 4;
  /** Listen charges its full matching initial tail and every delivery, including this writer. */
  readonly listener?: {
    readonly initialTail: 'all-matching-documents';
    readonly deliveries: 'including-own-writes';
    readonly catchUp: 'additional-rest-reads';
    readonly reconnects: 'fresh-query-cost';
  };
}
/** Caller owns reservation/persistence and the existing budget model, not this transport. */
export interface SyncAdmissionGuard {
  admit(cost: SyncAdmissionCost): Promise<void>;
  /** Provider document count: the complete initial server tail, then each delivered batch. */
  received?(documents: number): Promise<void>;
}
export const SYNC_PROVIDER_BYTES = 397312;
const unavailable = (): SyncFailure => ({ code: 'REMOTE_STATE_UNAVAILABLE' });
export function syncCallLive(call: SyncCall): void {
  if (call.signal.aborted || !Number.isFinite(call.deadlineAtMs) || Date.now() >= call.deadlineAtMs)
    throw unavailable();
}
/** Bound every injected await even if a fake or provider ignores its AbortSignal. */
export async function syncWait<T>(call: SyncCall, action: () => Promise<T>): Promise<T> {
  syncCallLive(call);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let abort: (() => void) | undefined;
  const end = new Promise<never>((_, reject) => {
    abort = () => reject(unavailable());
    call.signal.addEventListener('abort', abort, { once: true });
    timer = setTimeout(abort, Math.max(0, call.deadlineAtMs - Date.now()));
  });
  try {
    return await Promise.race([action(), end]);
  } finally {
    clearTimeout(timer);
    if (abort) call.signal.removeEventListener('abort', abort);
  }
}
/** Keep caller guard failures in the frozen refusal vocabulary; never expose its exception text. */
export async function admitSync(
  guard: SyncAdmissionGuard,
  cost: SyncAdmissionCost,
  call: SyncCall,
): Promise<void> {
  try {
    await syncWait(call, () => guard.admit(Object.freeze(cost)));
  } catch (error) {
    throw syncGuardFailure(error);
  }
}
/** Preserve only the frozen guard refusal, never caller exception text. */
export function syncGuardFailure(error: unknown): SyncFailure {
  if (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    error.code === 'REMOTE_BUDGET_EXHAUSTED' &&
    'resetAtMs' in error &&
    typeof error.resetAtMs === 'number' &&
    Number.isFinite(error.resetAtMs) &&
    error.resetAtMs >= 0 &&
    'retryAfterMs' in error &&
    typeof error.retryAfterMs === 'number' &&
    Number.isFinite(error.retryAfterMs) &&
    error.retryAfterMs >= 0
  )
    return {
      code: 'REMOTE_BUDGET_EXHAUSTED',
      resetAtMs: error.resetAtMs,
      retryAfterMs: error.retryAfterMs,
    };
  return unavailable();
}

/** The owner fence checks its uid/project/generation after every asynchronous boundary. */
export function syncHttp(
  context: SyncContext,
  guard: SyncAdmissionGuard,
  fetcher: typeof fetch,
  live: () => void,
) {
  const base = `https://firestore.googleapis.com/v1/projects/${context.config.projectId}/databases/(default)/documents`;
  return async (
    operation: 'read' | 'query' | 'commit',
    path: string,
    body: string | undefined,
    call: SyncCall,
    reads: number,
  ): Promise<{ kind: 'response'; status: number; value: unknown } | { kind: 'unknown' }> => {
    const check = () => {
      live();
      syncCallLive(call);
    };
    check();
    if (body !== undefined && new TextEncoder().encode(body).byteLength > SYNC_PROVIDER_BYTES)
      throw { code: 'REMOTE_INPUT_INVALID' };
    await admitSync(
      guard,
      {
        operation,
        reads,
        writes: operation === 'commit' ? 1 : 0,
        rulesLookupsPerDocument: operation === 'commit' ? WRITE_LOOKUPS : 3,
      },
      call,
    );
    check();
    const credential = await syncWait(call, () => context.auth.token(call));
    check();
    if ('state' in credential) throw credential;
    if (
      credential.projectId !== context.config.projectId ||
      credential.uid !== context.uid ||
      credential.expiresAtMs <= Date.now() ||
      !Number.isFinite(credential.expiresAtMs) ||
      !/^[\x21-\x7e]{1,16384}$/.test(credential.token)
    )
      throw unavailable();
    const controller = new AbortController();
    const abort = () => controller.abort();
    call.signal.addEventListener('abort', abort, { once: true });
    const timer = setTimeout(abort, Math.max(0, call.deadlineAtMs - Date.now()));
    let sent = false;
    try {
      check();
      const url =
        operation === 'read'
          ? `${base}/${path.split('/').map(encodeURIComponent).join('/')}`
          : `${base}${operation === 'query' && path ? '/' + path.split('/').map(encodeURIComponent).join('/') : ''}:${operation === 'query' ? 'runQuery' : 'commit'}`;
      sent = true;
      const response = await syncWait(call, () =>
        fetcher(url, {
          method: operation === 'read' ? 'GET' : 'POST',
          headers: {
            Authorization: `Bearer ${credential.token}`,
            'Content-Type': 'application/json',
          },
          body,
          signal: controller.signal,
          redirect: 'error',
          credentials: 'omit',
          cache: 'no-store',
        }),
      );
      check();
      if (controller.signal.aborted) throw unavailable();
      const reader = response.body?.getReader();
      if (!reader) throw unavailable();
      const chunks: Uint8Array[] = [];
      let length = 0;
      try {
        for (;;) {
          const chunk = await syncWait(call, () => reader.read());
          check();
          if (controller.signal.aborted) throw unavailable();
          if (chunk.done) break;
          length += chunk.value.byteLength;
          if (length > SYNC_PROVIDER_BYTES) throw unavailable();
          chunks.push(chunk.value);
        }
      } finally {
        void reader.cancel().catch(() => {});
        reader.releaseLock();
      }
      check();
      const bytes = new Uint8Array(length);
      let offset = 0;
      for (const chunk of chunks) {
        bytes.set(chunk, offset);
        offset += chunk.byteLength;
      }
      const value: unknown = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
      check();
      return { kind: 'response', status: response.status, value };
    } catch {
      if (operation === 'commit' && sent) return { kind: 'unknown' };
      throw unavailable();
    } finally {
      clearTimeout(timer);
      call.signal.removeEventListener('abort', abort);
    }
  };
}
