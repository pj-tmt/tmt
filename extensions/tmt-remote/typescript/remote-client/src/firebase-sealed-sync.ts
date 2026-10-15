/** Bound sealed-document owner. REST data and push hints share identity, never delivery progress. */
import {
  syncDocumentPath,
  validateSyncDocument,
  validateSyncFields,
  validateSyncInterest,
  validateSyncMutation,
  type SealedDocument,
  type SealedSyncPort,
  type SyncCall,
  type SyncContext,
  type SyncInterest,
  type SyncMutation,
  type SyncResource,
  type SyncSubscription,
  type SyncChange,
  type SyncFailure,
} from './sealed-sync.js';
import { classifyProviderExhausted } from './budget.js';
import { createSyncQuery } from './firebase-sync-query.js';
import {
  syncHttp,
  syncCallLive,
  syncWait,
  admitSync,
  syncGuardFailure,
  SYNC_PROVIDER_BYTES,
  type SyncAdmissionGuard,
} from './firebase-sync-http.js';

export interface SyncWakePort {
  listen(
    context: SyncContext,
    interest: SyncInterest,
    changed: (documents?: number) => void,
    failed: (failure?: SyncFailure) => void,
  ): Promise<() => void>;
}
const tab = { writes: 0, listeners: 0, streams: new Set<string>() };
const unavailable = () => ({ code: 'REMOTE_STATE_UNAVAILABLE' as const });
/** Identity fences retain only their typed scope verdict; private diagnostics never escape. */
const ownerFailure = (error: unknown): SyncFailure =>
  typeof error === 'object' &&
  error !== null &&
  'code' in error &&
  error.code === 'REMOTE_SCOPE_DENIED'
    ? { code: 'REMOTE_SCOPE_DENIED' }
    : unavailable();
const invalid = () => ({ code: 'REMOTE_INPUT_INVALID' as const });
const canonicalFields = (value: Record<string, unknown>) =>
  JSON.stringify(
    Object.fromEntries(Object.entries(value).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))),
  );
const digest = async (bytes: Uint8Array) =>
  Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new Uint8Array(bytes))), (b) =>
    b.toString(16).padStart(2, '0'),
  ).join('');

export function createSealedSync(
  context: SyncContext,
  guard: SyncAdmissionGuard,
  fetcher: typeof fetch,
  wake: SyncWakePort,
  ownerLive: () => void,
  page: { listeners: number } = { listeners: 0 },
  onClose: () => void = () => {},
): SealedSyncPort {
  let closed = false;
  const subscriptions = new Set<SyncSubscription>();
  const calls = new Set<AbortController>();
  const live = () => {
    ownerLive();
    if (closed) throw unavailable();
  };
  const http = syncHttp(context, guard, fetcher, live);
  const base = `projects/${context.config.projectId}/databases/(default)/documents/`;
  const resource = (name: string): SyncResource => {
    const r = context.resources.find((r) => r.name === name);
    if (!r) throw invalid();
    return r;
  };
  const invoke = async <T>(call: SyncCall, action: (bound: SyncCall) => Promise<T>): Promise<T> => {
    live();
    syncCallLive(call);
    const controller = new AbortController();
    const abort = () => controller.abort();
    call.signal.addEventListener('abort', abort, { once: true });
    calls.add(controller);
    try {
      return await action({ ...call, signal: controller.signal });
    } finally {
      calls.delete(controller);
      call.signal.removeEventListener('abort', abort);
    }
  };
  const document = async (value: unknown, r: SyncResource): Promise<SealedDocument> => {
    if (
      typeof value !== 'object' ||
      value === null ||
      !('name' in value) ||
      !('fields' in value) ||
      typeof value.name !== 'string' ||
      typeof value.fields !== 'object' ||
      value.fields === null ||
      Array.isArray(value.fields)
    )
      throw unavailable();
    const prefix = `${base}x/${context.extension}/${r.path}/`;
    if (!value.name.startsWith(prefix)) throw unavailable();
    const id = value.name.slice(prefix.length);
    const bytes = new TextEncoder().encode(
      canonicalFields(value.fields as Record<string, unknown>),
    );
    const d = { target: { resource: r.name, id }, bytes, digest: await digest(bytes) };
    live();
    await validateSyncDocument(context, d, 'read');
    live();
    return d;
  };
  const status = (code: number): never => {
    if (code === 401 || code === 403) throw { code: 'REMOTE_SCOPE_DENIED' };
    if (code === 429) {
      const result = classifyProviderExhausted({ mutation: false }, Date.now()).outcome;
      if (result.outcome === 'refused')
        throw {
          code: 'REMOTE_BUDGET_EXHAUSTED',
          resetAtMs: result.resetAtMs,
          retryAfterMs: result.retryAfterMs,
        };
    }
    throw unavailable();
  };
  const runQuery = createSyncQuery(context, http, resource, document, status, live);
  const port: SealedSyncPort = {
    context,
    read(target, call) {
      const path = syncDocumentPath(context, target);
      const r = resource(target.resource);
      if (!r.sync) throw unavailable();
      return invoke(call, async (bound) => {
        const response = await http('read', path, undefined, bound, 1);
        if (response.kind === 'unknown') throw unavailable();
        if (response.status === 404) return undefined;
        if (response.status !== 200) status(response.status);
        const d = await document(response.value, r);
        if (d.target.id !== target.id) throw unavailable();
        syncCallLive(bound);
        return d;
      });
    },
    query(request, call) {
      return invoke(call, (bound) => runQuery(request, bound));
    },
    async publish(mutation: SyncMutation, call) {
      const desired =
        mutation.mode === 'delete'
          ? structuredClone(mutation)
          : {
              mode: mutation.mode,
              document: {
                ...mutation.document,
                target: { ...mutation.document.target },
                bytes: new Uint8Array(mutation.document.bytes),
              },
            };
      await validateSyncMutation(context, desired);
      live();
      syncCallLive(call);
      const target = desired.mode === 'delete' ? desired.target : desired.document.target;
      const r = resource(target.resource);
      const fields =
        desired.mode === 'delete' ? undefined : validateSyncFields(desired.document.bytes);
      const stream =
        fields && r.sync!.query
          ? (fields[r.sync!.query.streamField] as { stringValue: string }).stringValue
          : target.id;
      const streamKey = JSON.stringify([context.config.projectId, context.uid, stream]);
      if (tab.streams.has(streamKey) || tab.writes >= 4) throw { code: 'REMOTE_RATE_LIMITED' };
      tab.streams.add(streamKey);
      tab.writes++;
      try {
        const name = `${base}${syncDocumentPath(context, target)}`;
        const write =
          desired.mode === 'delete'
            ? { delete: name }
            : {
                update: { name, fields },
                ...(desired.mode === 'create' ? { currentDocument: { exists: false } } : {}),
              };
        return await invoke(call, async (bound) => {
          const response = await http(
            'commit',
            '',
            JSON.stringify({ writes: [write] }),
            bound,
            0,
            Object.freeze({ mode: desired.mode, resource: target.resource, target: target.id }),
          );
          if (response.kind === 'unknown') return response;
          if (
            response.status === 403 &&
            typeof response.value === 'object' &&
            response.value !== null &&
            'error' in response.value &&
            typeof response.value.error === 'object' &&
            response.value.error !== null &&
            'code' in response.value.error &&
            response.value.error.code === 403 &&
            'status' in response.value.error &&
            response.value.error.status === 'PERMISSION_DENIED'
          )
            throw { code: 'REMOTE_SCOPE_DENIED' };
          if (
            response.status === 409 &&
            desired.mode === 'create' &&
            typeof response.value === 'object' &&
            response.value !== null &&
            'error' in response.value &&
            typeof response.value.error === 'object' &&
            response.value.error !== null &&
            'status' in response.value.error &&
            response.value.error.status === 'ALREADY_EXISTS'
          ) {
            const existing = await http(
              'read',
              syncDocumentPath(context, target),
              undefined,
              bound,
              1,
            );
            if (existing.kind === 'unknown' || existing.status !== 200) return { kind: 'unknown' };
            const actual = await document(existing.value, r);
            live();
            syncCallLive(bound);
            if (actual.digest === desired.document.digest) return { kind: 'already-present' };
            throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'conflict' };
          }
          if (
            response.status === 200 &&
            typeof response.value === 'object' &&
            response.value !== null &&
            'writeResults' in response.value &&
            Array.isArray(response.value.writeResults) &&
            response.value.writeResults.length === 1 &&
            'commitTime' in response.value &&
            typeof response.value.commitTime === 'string' &&
            Number.isFinite(Date.parse(response.value.commitTime))
          )
            return { kind: 'confirmed' };
          return { kind: 'unknown' };
        });
      } finally {
        tab.streams.delete(streamKey);
        tab.writes--;
      }
    },
    async subscribe(interest, call) {
      const boundInterest = structuredClone(interest);
      validateSyncInterest(context, boundInterest);
      if (
        new TextEncoder().encode(JSON.stringify(boundInterest.scope)).byteLength >
        SYNC_PROVIDER_BYTES
      )
        throw invalid();
      live();
      syncCallLive(call);
      if (page.listeners >= 3 || tab.listeners >= 12) throw { code: 'REMOTE_RATE_LIMITED' };
      let ended = false,
        dirty = false,
        failure: SyncFailure | undefined;
      let pending: { resolve(change: SyncChange): void; reject(error: unknown): void } | undefined;
      let stop: (() => void) | undefined;
      const publish = () => {
        if (ended || closed) return;
        try {
          live();
        } catch (error) {
          failure ??= ownerFailure(error);
        }
        if (pending) {
          const p = pending;
          pending = undefined;
          if (failure) p.reject(failure);
          else {
            dirty = false;
            p.resolve({ kind: 'changed' });
          }
        }
      };
      const subscription: SyncSubscription = {
        async next(nextCall) {
          if (failure) throw failure;
          if (ended || closed) throw unavailable();
          try {
            live();
          } catch (error) {
            throw ownerFailure(error);
          }
          if (nextCall.signal.aborted)
            throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' };
          if (!Number.isFinite(nextCall.deadlineAtMs)) throw unavailable();
          if (Date.now() >= nextCall.deadlineAtMs)
            throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'deadline' };
          if (pending) return Promise.reject({ code: 'REMOTE_RATE_LIMITED' });
          if (dirty) {
            dirty = false;
            return Promise.resolve({ kind: 'changed' });
          }
          return new Promise((resolve, reject) => {
            const controller = setTimeout(
              () => {
                pending?.reject({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'deadline' });
                pending = undefined;
              },
              Math.max(0, nextCall.deadlineAtMs - Date.now()),
            );
            const abort = () => {
              pending?.reject({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
              pending = undefined;
            };
            nextCall.signal.addEventListener('abort', abort, { once: true });
            const cleanup = () => {
              clearTimeout(controller);
              nextCall.signal.removeEventListener('abort', abort);
            };
            pending = {
              resolve(change) {
                cleanup();
                resolve(change);
              },
              reject(error) {
                cleanup();
                reject(error);
              },
            };
          });
        },
        close() {
          if (ended) return;
          ended = true;
          stop?.();
          subscriptions.delete(subscription);
          tab.listeners--;
          page.listeners--;
          pending?.reject(failure ?? unavailable());
          pending = undefined;
        },
      };
      subscriptions.add(subscription);
      tab.listeners++;
      page.listeners++;
      try {
        await admitSync(
          guard,
          {
            operation: 'listen',
            reads: null,
            writes: 0,
            rulesLookupsPerDocument: 3,
            listener: {
              initialTail: 'all-matching-documents',
              deliveries: 'including-own-writes',
              catchUp: 'additional-rest-reads',
              reconnects: 'fresh-query-cost',
            },
          },
          call,
        );
        live();
        syncCallLive(call);
        const credential = await syncWait(call, () => context.auth.token(call));
        live();
        syncCallLive(call);
        if ('state' in credential) throw credential;
        if (
          credential.projectId !== context.config.projectId ||
          credential.uid !== context.uid ||
          !Number.isFinite(credential.expiresAtMs) ||
          credential.expiresAtMs <= Date.now()
        )
          throw unavailable();
        stop = await syncWait(call, async () => {
          const detach = await wake.listen(
            context,
            boundInterest,
            (documents) => {
              if (ended || closed || failure) return;
              const changed = () => {
                if (ended || closed || failure) return;
                dirty = true;
                publish();
              };
              const refused = (error: unknown) => {
                if (ended || closed) return;
                failure = syncGuardFailure(error);
                publish();
                subscription.close();
              };
              if (!guard.received) return changed();
              try {
                if (
                  typeof documents !== 'number' ||
                  !Number.isSafeInteger(documents) ||
                  documents < 0
                )
                  throw invalid();
                // Charge every batch even when Changed coalesces; the caller owns its ledger.
                void guard.received(documents).then(changed, refused);
              } catch (error) {
                refused(error);
              }
            },
            (error) => {
              failure = error ?? unavailable();
              publish();
              subscription.close();
            },
          );
          if (ended || closed || call.signal.aborted || Date.now() >= call.deadlineAtMs) {
            detach();
            throw failure ?? unavailable();
          }
          return detach;
        });
        live();
        syncCallLive(call);
        if (ended) throw failure ?? unavailable();
        return subscription;
      } catch (error) {
        subscription.close();
        throw error;
      }
    },
    close() {
      if (closed) return;
      closed = true;
      for (const controller of calls) controller.abort();
      for (const s of subscriptions) s.close();
      onClose();
    },
  };
  return Object.freeze(port);
}
