import { ASK_OBSERVATION_MS } from './ask-records.js';
import { decodeText, requireValue, text } from '@tmt/colab-client';
import { FrozenAsk, type AdmittedSelection, type AskDestination } from './ask-intent.js';
import {
  refusalReason,
  SessionEndedError,
  SessionEvictedError,
  ReadRefusedError,
  sessionEndedReason,
  sessionEviction,
  type RemoteClient,
  type RemoteAgent,
  type RemoteContext,
  type RemoteRefusalCode,
  type SessionEndCode,
  type ResultState,
  type SendState,
} from './ask-remote.js';
import { AskRecordStore } from './ask-record-store.js';
import { ASK_MESSAGE_BYTES, ASK_REPLY_BYTES, type AskLedgerView } from './ask-records.js';
const ACTIVATION_READ_LIMIT = 256;
export interface AskDestinations {
  context: RemoteContext;
  machines: { id: string; name: string; online: 'online'; agents: RemoteAgent[] }[];
}
export type DirectoryReadPhase = 'session' | 'directory';
export type DirectoryReadFailure = {
  kind: 'unavailable';
  phase: DirectoryReadPhase;
  failure: 'ended' | 'evicted' | 'refused' | 'unavailable';
  code?: RemoteRefusalCode | 'REMOTE_REFUSED' | SessionEndCode;
};
export type AskDirectoryObservation =
  | { kind: 'ready'; checkedAt: number; snapshot: AskDestinations }
  | DirectoryReadFailure;
export interface AskControllerOptions {
  store: AskRecordStore;
  remote: RemoteClient;
  key: CryptoKey;
  selection(): AdmittedSelection;
  sessionEnded?(error?: SessionEvictedError): void;
}
/** Trusted parent composition: explicit Send is the only Remote write. All
 * state/result reads operate on the current device's admitted immutable ledger.
 * Registration replaces the client and controller together after Session end;
 * this controller stops observing and never carries an old preview across it. */
export class AskController {
  #destinations: AskDestinations | null = null;
  #sending = new Map<string, { preview: FrozenAsk; task: Promise<AskLedgerView> }>();
  #observing: Promise<void> | null = null;
  #ended = false;
  #endSession(error?: SessionEvictedError) {
    if (this.#ended) return;
    this.#ended = true;
    this.options.sessionEnded?.(error);
  }
  constructor(private options: AskControllerOptions) {}
  /** One current-session/list normalization owner. Observation never updates the
   * Ask destination cache or invokes the session lifecycle callback. */
  async #readDestinations(phase: (value: DirectoryReadPhase) => void): Promise<AskDestinations> {
    const { remote, store } = this.options;
    phase('session');
    requireValue(!this.#ended);
    const context = await remote.context();
    const validate = () =>
      requireValue(
        !this.#ended &&
          context.deviceId === store.scope.deviceId &&
          (context.expiresAtMs === null || Date.now() < context.expiresAtMs),
      );
    validate();
    phase('directory');
    const agents = await remote.listAgents();
    phase('session');
    validate();
    return {
      context,
      machines: [{ id: context.machineId, name: 'This machine', online: 'online', agents }],
    };
  }
  async observeDestinations(): Promise<AskDirectoryObservation> {
    let phase: DirectoryReadPhase = 'session';
    try {
      const snapshot = await this.#readDestinations((value) => {
        phase = value;
      });
      return { kind: 'ready', checkedAt: Date.now(), snapshot: structuredClone(snapshot) };
    } catch (error) {
      if (error instanceof SessionEvictedError)
        return { kind: 'unavailable', phase, failure: 'evicted', code: 'REMOTE_SESSION_EVICTED' };
      if (error instanceof SessionEndedError)
        return {
          kind: 'unavailable',
          phase,
          failure: error.code === 'REMOTE_SEQUENCE_UNAVAILABLE' ? 'unavailable' : 'ended',
          code: error.code,
        };
      if (error instanceof ReadRefusedError)
        return { kind: 'unavailable', phase, failure: 'refused', code: error.code };
      return { kind: 'unavailable', phase, failure: 'unavailable' };
    }
  }
  async destinations(): Promise<AskDestinations> {
    try {
      this.#destinations = await this.#readDestinations(() => {});
      return structuredClone(this.#destinations);
    } catch (error) {
      if (error instanceof SessionEvictedError) this.#endSession(error);
      else if (error instanceof SessionEndedError) this.#endSession();
      throw error;
    }
  }
  /** Synchronous capture; no await can replace the parent's chosen source. */
  prepare(destination: AskDestination, options: Parameters<typeof FrozenAsk.capture>[2] = {}) {
    requireValue(!this.#ended);
    const selection = this.options.selection();
    const snapshot = this.#destinations;
    requireValue(snapshot !== null);
    const c = snapshot.context;
    const agent = snapshot.machines[0].agents.find((v) => v.id === destination.agent);
    requireValue(
      agent !== undefined &&
        destination.machine === c.machineId &&
        destination.grantExpiresAt === c.expiresAtMs &&
        destination.grantRevision === c.grantRevision &&
        destination.deviceName === c.deviceName,
    );
    requireValue(
      selection.space === this.options.store.scope.space &&
        selection.page === this.options.store.scope.page &&
        selection.senderDevice === c.deviceId,
    );
    return FrozenAsk.capture(
      selection,
      {
        ...destination,
        agentName: agent.name,
        machineName: snapshot.machines[0].name,
        online: 'online',
        mode: c.mode,
      },
      {
        ...options,
        inputLimit: Math.min(options.inputLimit ?? ASK_MESSAGE_BYTES, ASK_MESSAGE_BYTES),
      },
    );
  }
  send(preview: FrozenAsk): Promise<AskLedgerView> {
    const id = preview.view.operationId;
    const previous = this.#sending.get(id);
    if (previous) {
      if (previous.preview === preview) return previous.task;
      return this.options.store.exclusive(id, async () => {
        await this.options.store.adopt(await preview.signed(this.options.key), preview.view);
        return this.options.store.view(id);
      });
    }
    const task = this.options.store.exclusive(id, async () => {
      const { remote, store, key } = this.options;
      let started = false,
        adopted = false,
        sessionEnd = false,
        evicted: SessionEvictedError | null = null;
      try {
        requireValue(!this.#ended);
        const current = await remote.context(),
          view = preview.view;
        requireValue(
          current.deviceId === store.scope.deviceId &&
            current.expiresAtMs === view.grantExpiresAt &&
            current.machineId === view.machine &&
            current.grantRevision === view.grantRevision &&
            current.deviceName === view.deviceName &&
            (current.expiresAtMs === null || Date.now() < current.expiresAtMs),
        );
        const signed = await preview.signed(key);
        const adoption = await store.adopt(signed, view);
        adopted = true;
        if (adoption === 'existing') return store.view(id);
        await store.state(id, 'dispatching');
        // Publication may outlast preview validity; expiry still has no effect.
        if (Date.now() >= preview.expiresAt)
          return store.state(id, 'expired', null, 'INTENT_EXPIRED');
        // Reopening/adopting never dispatches. This is still the original click,
        // with the pinned session/grant checked again after durable publication.
        const fence = await remote.context();
        requireValue(
          fence.deviceId === current.deviceId &&
            fence.expiresAtMs === current.expiresAtMs &&
            fence.machineId === current.machineId &&
            fence.grantRevision === current.grantRevision &&
            fence.deviceName === current.deviceName &&
            (fence.expiresAtMs === null || Date.now() < fence.expiresAtMs) &&
            Date.now() < preview.expiresAt,
        );
        started = true;
        const result = await remote.send({
          operationId: id,
          agentId: view.agent,
          message: decodeText(preview.finalBytes()),
        });
        requireValue(result.operationId === id);
        evicted = sessionEviction(result) ?? null;
        sessionEnd =
          (result.state === 'uncertain' && sessionEndedReason(result.reason)) ||
          (result.state === 'refused' && result.reason === 'REMOTE_SESSION_ENDED');
        const updated = await store.state(
          id,
          result.state,
          result.state === 'accepted' ? result.requestId : null,
          result.state === 'refused'
            ? refusalReason(result.reason)
            : result.state === 'cancelled'
              ? 'REMOTE_CANCELLED'
              : result.state === 'uncertain'
                ? (result.reason ?? null)
                : null,
        );
        return updated;
      } catch (error) {
        if (error instanceof SessionEvictedError) {
          sessionEnd = true;
          evicted = error;
        } else if (error instanceof SessionEndedError) sessionEnd = true;
        if (!adopted) throw error;
        return await store.state(
          id,
          started || sessionEnd ? 'uncertain' : 'failed',
          null,
          evicted
            ? 'REMOTE_SESSION_EVICTED'
            : sessionEnd
              ? 'REMOTE_SESSION_ENDED'
              : started
                ? 'REMOTE_UNCERTAIN'
                : 'SEND_UNAVAILABLE',
        );
      } finally {
        if (evicted) this.#endSession(evicted);
        else if (sessionEnd) this.#endSession();
      }
    });
    this.#sending.set(id, { preview, task });
    return task;
  }
  recover(id: string): Promise<AskLedgerView> {
    return this.options.store.exclusive(id, async () => {
      requireValue(!this.#ended);
      const { store, remote } = this.options;
      let view = await store.view(id);
      if (['abandoned', 'failed', 'expired', 'refused', 'cancelled'].includes(view.state))
        return view;
      if (view.state !== 'accepted') {
        if (view.state === 'dispatching')
          view = await store.state(id, 'uncertain', null, 'OBSERVATION_INTERRUPTED');
        let state: SendState;
        try {
          state = await remote.operation(id);
        } catch (error) {
          if (error instanceof SessionEvictedError) this.#endSession(error);
          else if (error instanceof SessionEndedError) {
            if (error.code === 'REMOTE_SEQUENCE_UNAVAILABLE')
              await store.state(id, 'uncertain', view.requestId, error.code);
            this.#endSession();
          }
          throw error;
        }
        requireValue(state.operationId === id);
        if (state.state === 'refused') {
          const eviction = sessionEviction(state);
          if (eviction) {
            this.#endSession(eviction);
            throw eviction;
          }
          if (state.reason === 'REMOTE_SESSION_ENDED') {
            this.#endSession();
            throw new SessionEndedError('REMOTE_SESSION_ENDED');
          }
          throw new ReadRefusedError(refusalReason(state.reason));
        }
        view = await store.state(
          id,
          state.state,
          state.state === 'accepted' ? state.requestId : null,
          state.state === 'cancelled'
            ? 'REMOTE_CANCELLED'
            : state.state === 'uncertain'
              ? (state.reason ?? (view.state === 'dispatching' ? 'OBSERVATION_INTERRUPTED' : null))
              : null,
        );
        if (state.state === 'uncertain' && sessionEndedReason(state.reason)) {
          this.#endSession();
          return view;
        }
      }
      if (view.state !== 'accepted' || view.reply) return view;
      requireValue(view.requestId !== null);
      let result: ResultState;
      try {
        result = await remote.result(view.requestId);
      } catch (error) {
        if (error instanceof SessionEvictedError) this.#endSession(error);
        else if (error instanceof SessionEndedError) this.#endSession();
        throw error;
      }
      requireValue(result.requestId === view.requestId);
      if (result.state === 'replied') {
        if (text(result.message).length > ASK_REPLY_BYTES)
          return store.state(id, 'accepted', view.requestId, 'REPLY_TOO_LARGE');
        return store.reply(id, view.requestId, result.message);
      }
      if (result.state === 'unavailable')
        return store.state(id, 'accepted', view.requestId, 'RESULT_UNAVAILABLE');
      return view;
    });
  }
  /** A bounded visible-page observer. It calls only receipt/final reads and
   * owns no send capability on reload, reconnect or a timer wake. */
  observe(signal: AbortSignal, refreshOlder = false): Promise<void> {
    if (this.#ended) return Promise.resolve();
    if (this.#observing) return this.#observing;
    const started = performance.now();
    const unresolved = (view: AskLedgerView) =>
      !view.reply &&
      ['dispatching', 'held', 'accepted', 'uncertain'].includes(view.state) &&
      !['RESULT_UNAVAILABLE', 'REPLY_TOO_LARGE'].includes(view.reason ?? '');
    const withinHorizon = (view: AskLedgerView) =>
      unresolved(view) && Date.now() - view.intent.issuedAt < ASK_OBSERVATION_MS;
    const task = (async () => {
      for (
        let cycle = 0;
        !signal.aborted && !this.#ended && performance.now() - started < ASK_OBSERVATION_MS;
        cycle++
      ) {
        if (typeof document !== 'undefined' && document.visibilityState === 'hidden') {
          await new Promise<void>((resolve) => {
            const wake = () => {
              if (
                !signal.aborted &&
                document.visibilityState === 'hidden' &&
                performance.now() - started < ASK_OBSERVATION_MS
              )
                return;
              clearTimeout(timer);
              document.removeEventListener('visibilitychange', wake);
              signal.removeEventListener('abort', wake);
              resolve();
            };
            const timer = setTimeout(
              wake,
              Math.max(0, ASK_OBSERVATION_MS - (performance.now() - started)),
            );
            document.addEventListener('visibilitychange', wake);
            signal.addEventListener('abort', wake, { once: true });
            wake();
          });
          continue;
        }
        // Activation gets one bounded sequential catch-up pass, newest first.
        // Older asks never enter the retry loop; excess history keeps explicit recheck.
        const views = await this.options.store.views();
        const pending = refreshOlder
          ? views
              .filter(unresolved)
              .sort(
                (a, b) =>
                  b.intent.issuedAt - a.intent.issuedAt ||
                  a.intent.operationId.localeCompare(b.intent.operationId),
              )
              .slice(0, ACTIVATION_READ_LIMIT)
          : views.filter(withinHorizon);
        refreshOlder = false;
        if (!pending.length) return;
        for (const view of pending) {
          if (
            signal.aborted ||
            this.#ended ||
            performance.now() - started >= ASK_OBSERVATION_MS ||
            (typeof document !== 'undefined' && document.visibilityState === 'hidden')
          )
            break;
          try {
            await this.recover(view.intent.operationId);
          } catch {
            // Read/publication failure leaves the original operation for re-check.
          }
        }
        if (this.#ended || !(await this.options.store.views()).some(withinHorizon)) return;
        if (!signal.aborted)
          await new Promise<void>((resolve) => {
            const done = () => {
              clearTimeout(timer);
              signal.removeEventListener('abort', done);
              resolve();
            };
            const timer = setTimeout(done, Math.min(2000 * 2 ** Math.min(cycle, 4), 30000));
            signal.addEventListener('abort', done, { once: true });
            if (signal.aborted) done();
          });
      }
    })().finally(() => {
      if (this.#observing === task) this.#observing = null;
    });
    this.#observing = task;
    return task;
  }
  abandon(id: string): Promise<AskLedgerView> {
    return this.options.store.exclusive(id, async () => {
      const view = await this.options.store.view(id);
      requireValue(view.state === 'uncertain');
      return this.options.store.state(id, 'abandoned', view.requestId, 'MAY_HAVE_BEEN_DELIVERED');
    });
  }
}
