import { coreId, decodeText, requireValue, text } from '@tmt/colab-client';
import {
  FrozenAsk,
  type AdmittedSelection,
  type AskDestination,
  type SignedAsk,
} from './ask-intent.js';
import type { RemoteClient, RemoteAgent, RemoteContext, SendState } from './ask-remote.js';
import { record } from './storage.js';

export type StoredAskDraft = Pick<SignedAsk, 'input' | 'signature'>;
export type DraftAdoption = (intent: SignedAsk) => Promise<'created' | 'existing'>;
export type AskState =
  | SendState
  | {
      state: 'preview' | 'preparing' | 'failed';
      operationId: string;
    };

/** Immutable signed metadata only; final message bytes stay in attempt memory.
 * This record is not the native bridge ledger or encrypted own stream.
 * Web Locks serialize same-operation adoption across tabs; a stored draft never
 * authorizes a send on reload. Transaction completion precedes any port call. */
export const storeAskDraft: DraftAdoption = async (intent) => {
  const key = `ask:${intent.senderDevice}:${intent.operationId}`;
  return await navigator.locks.request(key, async () => {
    const previous = await record<StoredAskDraft>(key);
    if (previous) {
      if (previous.input !== intent.input || previous.signature !== intent.signature)
        throw new Error('INTENT_CONFLICT');
      return 'existing';
    }
    await record<StoredAskDraft>(key, { input: intent.input, signature: intent.signature });
    return 'created';
  });
};

/** One explicit attempt. Observing/reconstructing this object has no effect.
 * There is deliberately no retry: remote child-cleanup evidence is not available. */
export class AskAttempt {
  #pending?: Promise<AskState>;
  #state: AskState;
  constructor(
    readonly preview: FrozenAsk,
    private key: CryptoKey,
    private adopt: DraftAdoption = storeAskDraft,
    private remote?: RemoteClient,
  ) {
    this.#state = { state: 'preview', operationId: preview.view.operationId };
    Object.freeze(this);
  }
  get available(): boolean {
    return this.remote !== undefined;
  }
  get state(): Readonly<AskState> {
    return Object.freeze({ ...this.#state });
  }
  send(): Promise<AskState> {
    if (!this.available) return Promise.reject(new Error('Remote operations are unavailable.'));
    if (this.#pending) return this.#pending;
    this.#state = { state: 'preparing', operationId: this.preview.view.operationId };
    this.#pending = this.#send();
    return this.#pending;
  }
  async #send(): Promise<AskState> {
    const operationId = this.preview.view.operationId;
    let started = false;
    try {
      const intent = await this.preview.signed(this.key);
      if ((await this.adopt(intent)) === 'existing') {
        this.#state = { state: 'uncertain', operationId };
      } else {
        // Storage/signing may have outlasted validity; fail before the port call.
        requireValue(Date.now() < this.preview.expiresAt);
        started = true;
        const reply = await this.remote!.send({
          operationId,
          agentId: this.preview.view.agent,
          message: decodeText(this.preview.finalBytes()),
        });
        requireValue(
          reply.operationId === operationId &&
            ['held', 'accepted', 'uncertain', 'refused', 'cancelled'].includes(reply.state),
        );
        if (reply.state === 'accepted') {
          requireValue(typeof reply.requestId === 'string' && reply.requestId.startsWith('req_'));
          coreId(reply.requestId.slice(4));
        }
        this.#state = { ...reply };
      }
    } catch {
      this.#state = { state: started ? 'uncertain' : 'failed', operationId };
    }
    return this.state;
  }
}

import { AskRecordStore } from './ask-record-store.js';
import { ASK_MESSAGE_BYTES, ASK_REPLY_BYTES, type AskLedgerView } from './ask-records.js';
export interface AskDestinations {
  context: RemoteContext;
  machines: { id: string; name: string; online: 'online'; agents: RemoteAgent[] }[];
}
export interface AskControllerOptions {
  store: AskRecordStore;
  remote: RemoteClient;
  key: CryptoKey;
  selection(): AdmittedSelection;
}
/** Trusted parent composition: explicit Send is the only Remote write. All
 * state/result reads operate on the current device's admitted immutable ledger. */
export class AskController {
  #destinations: AskDestinations | null = null;
  #sending = new Map<string, { preview: FrozenAsk; task: Promise<AskLedgerView> }>();
  #observing: Promise<void> | null = null;
  constructor(private options: AskControllerOptions) {}
  async destinations(): Promise<AskDestinations> {
    const { remote, store } = this.options;
    const context = await remote.context();
    requireValue(
      context.deviceId === store.scope.deviceId &&
        (context.expiresAtMs === null || Date.now() < context.expiresAtMs),
    );
    const agents = await remote.listAgents();
    this.#destinations = {
      context,
      machines: [{ id: context.machineId, name: 'This machine', online: 'online', agents }],
    };
    return structuredClone(this.#destinations);
  }
  /** Synchronous capture; no await can replace the parent's chosen source. */
  prepare(destination: AskDestination, options: Parameters<typeof FrozenAsk.capture>[2] = {}) {
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
        delivery: agent.delivery,
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
        await this.options.store.adopt(await preview.signed(this.options.key));
        return this.options.store.view(id);
      });
    }
    const task = this.options.store.exclusive(id, async () => {
      const { remote, store, key } = this.options;
      let started = false,
        adopted = false;
      try {
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
        const adoption = await store.adopt(signed);
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
        await store.state(
          id,
          result.state,
          result.state === 'accepted' ? result.requestId : null,
          result.state === 'refused'
            ? 'REMOTE_REFUSED'
            : result.state === 'cancelled'
              ? 'REMOTE_CANCELLED'
              : null,
        );
        return store.view(id);
      } catch (error) {
        if (!adopted) throw error;
        return store.state(
          id,
          started ? 'uncertain' : 'failed',
          null,
          started ? 'REMOTE_UNCERTAIN' : 'SEND_UNAVAILABLE',
        );
      }
    });
    this.#sending.set(id, { preview, task });
    return task;
  }
  recover(id: string): Promise<AskLedgerView> {
    return this.options.store.exclusive(id, async () => {
      const { store, remote } = this.options;
      let view = await store.view(id);
      if (['abandoned', 'failed', 'expired', 'refused', 'cancelled'].includes(view.state))
        return view;
      if (view.state !== 'accepted') {
        if (view.state === 'dispatching')
          view = await store.state(id, 'uncertain', null, 'OBSERVATION_INTERRUPTED');
        const state = await remote.operation(id);
        requireValue(state.operationId === id);
        view = await store.state(
          id,
          state.state,
          state.state === 'accepted' ? state.requestId : null,
          state.state === 'refused'
            ? 'REMOTE_REFUSED'
            : state.state === 'cancelled'
              ? 'REMOTE_CANCELLED'
              : null,
        );
      }
      if (view.state !== 'accepted' || view.reply) return view;
      requireValue(view.requestId !== null);
      const result = await remote.result(view.requestId);
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
  observe(signal: AbortSignal): Promise<void> {
    if (this.#observing) return this.#observing;
    const started = performance.now();
    const unresolved = (view: AskLedgerView) =>
      !view.reply &&
      ['dispatching', 'held', 'accepted', 'uncertain'].includes(view.state) &&
      !['RESULT_UNAVAILABLE', 'REPLY_TOO_LARGE'].includes(view.reason ?? '') &&
      Date.now() - view.intent.issuedAt < 7200000;
    const task = (async () => {
      for (let cycle = 0; !signal.aborted && performance.now() - started < 7200000; cycle++) {
        if (typeof document !== 'undefined' && document.visibilityState === 'hidden') {
          await new Promise<void>((resolve) => {
            const wake = () => {
              if (
                !signal.aborted &&
                document.visibilityState === 'hidden' &&
                performance.now() - started < 7200000
              )
                return;
              clearTimeout(timer);
              document.removeEventListener('visibilitychange', wake);
              signal.removeEventListener('abort', wake);
              resolve();
            };
            const timer = setTimeout(wake, Math.max(0, 7200000 - (performance.now() - started)));
            document.addEventListener('visibilitychange', wake);
            signal.addEventListener('abort', wake, { once: true });
            wake();
          });
          continue;
        }
        const pending = (await this.options.store.views()).filter(unresolved);
        if (!pending.length) return;
        for (const view of pending) {
          if (
            signal.aborted ||
            performance.now() - started >= 7200000 ||
            (typeof document !== 'undefined' && document.visibilityState === 'hidden')
          )
            break;
          try {
            await this.recover(view.intent.operationId);
          } catch {
            // Read/publication failure leaves the original operation for re-check.
          }
        }
        if (!(await this.options.store.views()).some(unresolved)) return;
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
