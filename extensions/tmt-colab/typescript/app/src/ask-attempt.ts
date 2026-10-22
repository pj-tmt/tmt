import { ActivityTracker, type ActivityObservation } from './agent-activity.js';
import { ASK_OBSERVATION_MS } from './ask-records.js';
import {
  sameMessageState,
  type MessageState,
  type ObservedMessageState,
  type SendNowProgress,
} from './message-state.js';
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
  type SessionEndCode,
  type ResultState,
  type SendIdentity,
  type SendNowReply,
  type SendState,
} from './ask-remote.js';
import { AskRecordStore } from './ask-record-store.js';
import { OwnRecordConflict } from './own-publication.js';
import { ASK_MESSAGE_BYTES, ASK_REPLY_BYTES, type AskLedgerView } from './ask-records.js';
const ACTIVATION_READ_LIMIT = 256;
const ACTIVE_OBSERVATION_MS = 10 * 60 * 1000;
const ACTIVE_READ_STEP_MS = 3000;
const ACTIVITY_READ_INTERVAL_MS = 60_000;
export interface AskDestinations {
  context: RemoteContext;
  machines: { id: string; name: string; online: 'online'; agents: RemoteAgent[] }[];
}
export type DirectoryReadPhase = 'session' | 'directory';
export type DirectoryReadFailure = {
  kind: 'unavailable';
  phase: DirectoryReadPhase;
  failure: 'ended' | 'evicted' | 'refused' | 'unavailable';
  code?: ReadRefusedError['code'] | SessionEndCode;
};
export type AskDirectoryObservation =
  | { kind: 'ready'; checkedAt: number; snapshot: AskDestinations }
  | DirectoryReadFailure;
/** Send failed before any durable adoption or Remote write was attempted. */
export class UnadoptedAskError extends Error {
  constructor(cause: unknown) {
    super(cause instanceof Error ? cause.message : 'Ask unavailable', { cause });
    this.name = 'UnadoptedAskError';
  }
}
export interface AskControllerOptions {
  store: AskRecordStore;
  remote: RemoteClient;
  key: CryptoKey;
  selection(): AdmittedSelection;
  sessionEnded?(error?: SessionEvictedError): void;
  observationUnavailable?(unavailable: boolean): void;
  activityRead?(): Promise<AskDirectoryObservation>;
  activityObserved?(observation: ActivityObservation): void;
  /** Core's state of an accepted ask's message (#4317), or `undefined` once there is none to show.
   * Called only when what the page shows changes; the ledger never holds it. */
  messageStateObserved?(operationId: string, observed: ObservedMessageState | undefined): void;
  /** How Remote attributed a send it just admitted; informational, never changes the send (#4255). */
  identityNotice?(identity: SendIdentity): void;
}
/** Trusted parent composition: explicit Send is the only Remote write. All
 * state/result reads operate on the current device's admitted immutable ledger.
 * Registration replaces the client and controller together after Session end;
 * this controller stops observing and never carries an old preview across it. */
export class AskController {
  #destinations: AskDestinations | null = null;
  #sending = new Map<string, { preview: FrozenAsk; task: Promise<AskLedgerView> }>();
  #observing: Promise<void> | null = null;
  #observationWake: AbortController | null = null;
  #activeObservationUntil = 0;
  #ended = false;
  #activity = new ActivityTracker();
  #activityObservation: ActivityObservation | undefined;
  // Optional presentation reads never borrow from explicit Send admission.
  #nextActivityReadAt = 0;
  #directory: { snapshot: AskDestinations; receivedAt: number } | undefined;
  #intervalMs = 2000;
  #directoryRead: { task: Promise<AskDestinations>; phase: { value: DirectoryReadPhase } } | null =
    null;
  // What the page shows of Core's message state, by operation: a later read changes nothing
  // unless it differs, so a countdown costs one publication, not one per read.
  #messages = new Map<string, ObservedMessageState>();
  #showMessage(operationId: string, state: MessageState | undefined) {
    const shown = this.#messages.get(operationId);
    // What a Send now came to is about the state it was asked in: it goes when the state moves.
    const read = state && {
      ...state,
      receivedAt: performance.now(),
      ...(shown?.sendNow && shown.state === state.state ? { sendNow: shown.sendNow } : {}),
    };
    if (sameMessageState(shown, read)) return;
    if (read) this.#messages.set(operationId, read);
    else this.#messages.delete(operationId);
    this.options.messageStateObserved?.(operationId, read);
  }
  #showSendNow(operationId: string, progress: SendNowProgress | undefined) {
    const shown = this.#messages.get(operationId);
    if (!shown) return;
    const next: ObservedMessageState = { ...shown };
    if (progress) next.sendNow = progress;
    else delete next.sendNow;
    if (sameMessageState(shown, next)) return;
    this.#messages.set(operationId, next);
    this.options.messageStateObserved?.(operationId, next);
  }
  #clearMessages() {
    // Removing the key being visited is safe for a Map.
    for (const operationId of this.#messages.keys()) this.#showMessage(operationId, undefined);
  }
  #endSession(error?: SessionEvictedError) {
    if (this.#ended) return;
    this.#ended = true;
    this.#clearMessages();
    this.#observationWake?.abort();
    this.options.sessionEnded?.(error);
  }
  constructor(private options: AskControllerOptions) {}
  /** One current-session/list normalization owner. Observation never updates the
   * Ask destination cache or invokes the session lifecycle callback. */
  async #readDestinations(phase: (value: DirectoryReadPhase) => void): Promise<AskDestinations> {
    if (!this.#directoryRead) {
      const current = { value: 'session' as DirectoryReadPhase };
      const task = this.#acquireDirectory((value) => {
        current.value = value;
      }).finally(() => {
        if (this.#directoryRead?.task === task) this.#directoryRead = null;
      });
      this.#directoryRead = { task, phase: current };
    }
    const pending = this.#directoryRead;
    try {
      return await pending.task;
    } finally {
      phase(pending.phase.value);
    }
  }
  async #acquireDirectory(phase: (value: DirectoryReadPhase) => void): Promise<AskDestinations> {
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
    const snapshot: AskDestinations = {
      context,
      machines: [{ id: context.machineId, name: 'This machine', online: 'online', agents }],
    };
    this.#directory = { snapshot, receivedAt: performance.now() };
    return snapshot;
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
  send(preview: FrozenAsk, retryOf?: string | null): Promise<AskLedgerView> {
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
      let adoptionPhase: 'none' | 'pending' | 'complete' = 'none';
      let started = false,
        sessionEnd = false,
        evicted: SessionEvictedError | null = null;
      try {
        requireValue(!this.#ended);
        if (
          !this.#directory ||
          performance.now() - this.#directory.receivedAt >= 2 * this.#intervalMs
        )
          await this.observeDestinations();
        const before = this.#directory;
        if (before && performance.now() - before.receivedAt < 2 * this.#intervalMs) {
          const rows = before.snapshot.machines
            .filter((m) => m.id === preview.view.machine)
            .flatMap((m) => m.agents.filter((a) => a.id === preview.view.agent));
          if (rows.length === 1) this.#activity.anchor(id, rows[0].observedAtMs);
        }
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
        adoptionPhase = 'pending';
        const adoption = await store.adopt(signed, view, retryOf);
        adoptionPhase = 'complete';
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
        try {
          if (result.identity) this.options.identityNotice?.(result.identity);
        } catch {
          // The notice is informational: it never changes the send.
        }
        evicted = sessionEviction(result) ?? null;
        sessionEnd =
          (result.state === 'uncertain' && sessionEndedReason(result.reason)) ||
          (result.state === 'refused' && result.reason === 'REMOTE_SESSION_ENDED');
        const updated = await store.state(
          id,
          result.state,
          result.state === 'accepted' ? result.requestId : null,
          result.state === 'refused'
            ? refusalReason(result.reason, result.scope)
            : result.state === 'cancelled'
              ? 'REMOTE_CANCELLED'
              : result.state === 'uncertain'
                ? (result.reason ?? null)
                : null,
        );
        if (updated.state === 'accepted') {
          // Only a newly admitted explicit Send extends the fast read window.
          // Wake a sleeping or in-flight cycle; neither path dispatches again.
          this.#activeObservationUntil = performance.now() + ACTIVE_OBSERVATION_MS;
          this.#observationWake?.abort();
        }
        return updated;
      } catch (error) {
        if (error instanceof SessionEvictedError) {
          sessionEnd = true;
          evicted = error;
        } else if (error instanceof SessionEndedError) sessionEnd = true;
        if (adoptionPhase === 'none') throw new UnadoptedAskError(error);
        if (adoptionPhase === 'pending') throw error;
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
  /** Read the operation again and record what it shows. If another tab of this device already
   * wrote that record with a different value (its view of the operation was ahead of ours), the
   * own stream keeps the first value and nothing is written: this pass ends, and the next
   * observation reads the operation again. */
  async recover(id: string): Promise<AskLedgerView> {
    try {
      return await this.#recover(id);
    } catch (error) {
      if (error instanceof OwnRecordConflict) return this.options.store.view(id);
      throw error;
    }
  }
  #recover(id: string): Promise<AskLedgerView> {
    return this.options.store.exclusive(id, async () => {
      requireValue(!this.#ended);
      const { store, remote } = this.options;
      let view = await store.view(id);
      if (['abandoned', 'failed', 'expired', 'refused', 'cancelled'].includes(view.state)) {
        this.#showMessage(id, undefined);
        return view;
      }
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
      if (view.state !== 'accepted' || view.reply) {
        this.#showMessage(id, undefined);
        return view;
      }
      requireValue(view.requestId !== null);
      let result: ResultState;
      try {
        result = await remote.result(view.requestId);
      } catch (error) {
        // A read that fails cannot vouch for what was shown.
        this.#showMessage(id, undefined);
        if (error instanceof SessionEvictedError) this.#endSession(error);
        else if (error instanceof SessionEndedError) this.#endSession();
        throw error;
      }
      requireValue(result.requestId === view.requestId);
      this.#showMessage(id, result.state === 'pending' ? result.messageState : undefined);
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
  /**
   * Asks Core, through Remote, to type an accepted ask's message now (#4136). Only while the last
   * read of its state said it may be sent now; one ask however often it is pressed, never repeated
   * by Colab, and nothing of it is written to the ledger. The line then moves on from a read of
   * Core's state; what a refusal or an uncertain ask came to stays beside that state until it moves.
   */
  sendNow(id: string): Promise<SendNowReply> {
    const running = this.#askingNow.get(id);
    if (running) return running;
    const task = this.#askNow(id).finally(() => this.#askingNow.delete(id));
    this.#askingNow.set(id, task);
    return task;
  }
  #askingNow = new Map<string, Promise<SendNowReply>>();
  async #askNow(id: string): Promise<SendNowReply> {
    requireValue(!this.#ended);
    const { store, remote } = this.options;
    const view = await store.view(id);
    requireValue(
      view.state === 'accepted' &&
        !view.reply &&
        view.requestId !== null &&
        this.#messages.get(id)?.canSendNow === true,
    );
    this.#showSendNow(id, { phase: 'sending' });
    let reply: SendNowReply;
    try {
      reply = await remote.sendNow(view.requestId);
    } catch (error) {
      this.#showSendNow(id, undefined);
      if (error instanceof SessionEvictedError) this.#endSession(error);
      else if (error instanceof SessionEndedError) this.#endSession();
      throw error;
    }
    // What Remote read back is Core's state now; an ask that typed nothing leaves its outcome.
    if (reply.messageState) this.#showMessage(id, reply.messageState);
    this.#showSendNow(
      id,
      reply.state === 'sent' || reply.state === 'already'
        ? undefined
        : {
            phase: 'outcome',
            outcome: reply.state,
            ...(reply.reason === undefined ? {} : { reason: reply.reason }),
            ...(reply.reasonText === undefined ? {} : { reasonText: reply.reasonText }),
          },
    );
    try {
      await this.recover(id);
    } catch {
      // The reply stands; the next observation reads the state again.
    }
    return reply;
  }
  /** A bounded visible-page observer. It calls only receipt/final/inventory reads and
   * owns no send capability on reload, reconnect or a timer wake. */
  observe(signal: AbortSignal, refreshOlder = false): Promise<void> {
    if (this.#ended) return Promise.resolve();
    if (this.#observing) return this.#observing;
    const started = performance.now();
    const stopActivity = () => {
      if (this.#activityObservation) {
        this.#activityObservation = { ...this.#activityObservation, active: false };
        this.options.activityObserved?.(this.#activityObservation);
      }
    };
    signal.addEventListener('abort', stopActivity, { once: true });
    signal.addEventListener('abort', () => this.#clearMessages(), { once: true });
    let wakeOwner = new AbortController();
    this.#observationWake = wakeOwner;
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
        if (wakeOwner.signal.aborted) {
          cycle = 0;
          wakeOwner = new AbortController();
          this.#observationWake = wakeOwner;
        }
        // Capture before reads so a Send during a read also skips the stale sleep.
        const wakeSignal = wakeOwner.signal;
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
        if (!pending.length) {
          if (!signal.aborted && !this.#ended) this.options.observationUnavailable?.(false);
          return;
        }
        let unavailable = false;
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
            unavailable = true;
          }
        }
        if (
          !signal.aborted &&
          !this.#ended &&
          (typeof document === 'undefined' || document.visibilityState !== 'hidden')
        )
          this.options.observationUnavailable?.(unavailable);
        const remaining = (await this.options.store.views()).filter(withinHorizon);
        if (this.#ended || !remaining.length) return;
        if (!signal.aborted) {
          const idleStep = Math.min(2000 * 2 ** Math.min(cycle, 4), 30000);
          const step =
            performance.now() < this.#activeObservationUntil
              ? Math.min(idleStep, ACTIVE_READ_STEP_MS)
              : idleStep;
          this.#intervalMs = step;
          if (
            this.options.activityRead &&
            performance.now() >= this.#nextActivityReadAt &&
            (typeof document === 'undefined' || document.visibilityState !== 'hidden')
          ) {
            // Charge attempts before awaiting, including failures. A wake or re-observation
            // retains this controller's minimum acquisition interval.
            this.#nextActivityReadAt = performance.now() + ACTIVITY_READ_INTERVAL_MS;
            const observation = await this.options.activityRead().catch(() => undefined);
            const current = (await this.options.store.views()).filter(withinHorizon);
            if (!current.length || performance.now() - started >= ASK_OBSERVATION_MS) return;
            if (
              !signal.aborted &&
              !this.#ended &&
              (typeof document === 'undefined' || document.visibilityState !== 'hidden')
            ) {
              const snapshot = observation?.kind === 'ready' ? observation.snapshot : undefined;
              this.#activityObservation = {
                active: true,
                receivedAt: performance.now(),
                intervalMs: ACTIVITY_READ_INTERVAL_MS,
                agents:
                  snapshot?.machines.flatMap((machine) =>
                    machine.agents.map((agent) => ({
                      machine: machine.id,
                      machineName: machine.name,
                      agent: agent.id,
                      name: agent.name,
                      // The validated state needs no phase/owner clock; only elapsed copy does.
                      state: agent.activity?.state ?? 'unknown',
                    })),
                  ) ?? [],
                entries: current.map((view) => {
                  const operation = {
                    operationId: view.intent.operationId,
                    machine: view.intent.machine,
                    agent: view.intent.agent,
                  };
                  const rows =
                    snapshot?.machines
                      .filter((m) => m.id === operation.machine)
                      .flatMap((m) => m.agents.filter((a) => a.id === operation.agent)) ?? [];
                  const entry = this.#activity.read(
                    operation,
                    rows.length === 1 ? rows[0] : undefined,
                  );
                  return observation?.kind === 'ready'
                    ? entry
                    : { ...entry, kind: 'unknown' as const };
                }),
              };
              this.options.activityObserved?.(this.#activityObservation);
            }
          }
          await new Promise<void>((resolve) => {
            const done = () => {
              clearTimeout(timer);
              signal.removeEventListener('abort', done);
              wakeSignal.removeEventListener('abort', done);
              resolve();
            };
            const timer = setTimeout(done, step);
            signal.addEventListener('abort', done, { once: true });
            wakeSignal.addEventListener('abort', done, { once: true });
            if (signal.aborted || wakeSignal.aborted) done();
          });
        }
      }
    })().finally(() => {
      signal.removeEventListener('abort', stopActivity);
      stopActivity();
      if (this.#observing === task) {
        this.#observing = null;
        this.#observationWake = null;
      }
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
