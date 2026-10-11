import {
  ConnectionOwner,
  reconnectAttempt,
  type ConnectionOwnerOptions,
} from './connection-owner.js';
import { settle, terminal, type ConnectionState } from './connection-model.js';
import { PageReopener, ReadmitBudget, retryableReopen } from './readmit.js';

export interface ConnectionControllerOptions<Page> {
  /** The page's current binding; `reconnect` makes one fresh-session attempt. */
  binding(): { reconnect?(): Promise<boolean>; close(): void } | undefined;
  /** Reopen the page for its new epoch. */
  reopen(signal: AbortSignal): Promise<Page>;
  /** A reopen produced a new page: the caller swaps it in. */
  opened(next: Page): void;
  /** A failure that is not an epoch advance: still retrying, or ended (the caller may check for a newer build). */
  failed?(error: Error): void;
  /** Automatic reopens one page may use; shared by every controller of the same view. */
  budget?: ReadmitBudget;
  now?(): number;
  wait?: ConnectionOwnerOptions['wait'];
  visible?: ConnectionOwnerOptions['visible'];
  jitter?: ConnectionOwnerOptions['jitter'];
  reopenWait?: (ms: number, signal: AbortSignal) => Promise<void>;
}

/**
 * The one owner of an open page's connection (#2572): `connected`, `reconnecting` (silent, with
 * the page's drafts and open thread kept) or `ended` (one reason, one action). The page reports
 * every failure its binding publishes; this decides, retries and publishes the state every reader
 * shares. `Live` keeps its own bounded resync and stays the transport owner.
 */
const CONNECTED: ConnectionState = { phase: 'connected' };

export class ConnectionController<Page = unknown> {
  #state: ConnectionState = CONNECTED;
  readonly #listeners = new Set<() => void>();
  readonly #reopener = new PageReopener<Page>();
  readonly #budget: ReadmitBudget;
  #owner: ConnectionOwner | null = null;

  constructor(private readonly options: ConnectionControllerOptions<Page>) {
    this.#budget = options.budget ?? new ReadmitBudget();
  }

  get state(): ConnectionState {
    return this.#state;
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  /** A failure the page's binding published. An ended connection keeps its first ending. */
  report(error: Error): void {
    if (this.#state.phase === 'ended') return;
    const settled = settle(error);
    if (settled.phase === 'ended') return this.#end(error);
    if (settled.kind === 'epoch') return this.#reopen(error);
    // The reopen replaces this binding: a late transport failure from it changes nothing.
    if (this.#reopener.busy) return;
    // Nothing else drives a silent reconnect: a binding that cannot reconnect has ended.
    if (!this.#canReconnect()) return this.#end(error);
    this.options.failed?.(error);
    this.#set({ ...settled, error });
    this.#loop();
  }

  /** The page has a new binding: whatever the old one was doing no longer applies. */
  reset(): void {
    this.#owner?.stop();
    this.#owner = null;
    this.#set(CONNECTED);
  }

  /** The view is gone: stop every attempt and end the binding a reopen produced. */
  dispose(): void {
    this.#owner?.stop();
    this.#owner = null;
    this.#reopener.reset();
  }

  #set(next: ConnectionState): void {
    if (this.#state === next) return;
    this.#state = next;
    this.#listeners.forEach((listener) => listener());
  }

  #end(error: Error): void {
    this.options.failed?.(error);
    this.#owner?.stop();
    this.#owner = null;
    this.#reopener.reset();
    this.#set({ ...terminal(error), error });
  }

  /** Bind the new epoch without a reload, retrying a lagging one; a refusal or no budget ends. */
  #reopen(error: Error): void {
    if (this.#reopener.busy) return;
    if (!this.#budget.take(this.options.now?.() ?? Date.now())) return this.#end(error);
    // The reopen replaces the binding, so the old binding's loop must not keep driving it.
    this.#owner?.stop();
    this.#owner = null;
    this.#set({ phase: 'reconnecting', kind: 'epoch', error });
    this.#reopener.start((signal) => this.options.reopen(signal), {
      retryable: retryableReopen,
      wait: this.options.reopenWait,
      done: (result) => {
        if ('error' in result) return this.#end(result.error);
        this.options.binding()?.close();
        this.#set(CONNECTED);
        this.options.opened(result.value);
      },
    });
  }

  #canReconnect(): boolean {
    return Boolean(this.options.binding()?.reconnect);
  }

  /**
   * One reconnect loop at a time, for as long as the page's failure stays recoverable. A series
   * that gives up ends the connection with the failure it stopped on: with no button left,
   * nothing else would ever leave `reconnecting`.
   */
  #loop(): void {
    const binding = this.options.binding();
    if (this.#owner || !binding?.reconnect) return;
    const owner = new ConnectionOwner({
      attempt: reconnectAttempt({
        reconnect: async () => (await binding.reconnect?.()) ?? false,
        failure: () => (this.#state.phase === 'connected' ? null : this.#state.error),
        recovered: (interrupted) => {
          if (this.#state.phase !== 'connected' && this.#state.error === interrupted)
            this.#set(CONNECTED);
        },
        recoverable: (failure) => {
          if (failure === null) return false;
          const settled = settle(failure);
          return settled.phase === 'reconnecting' && settled.kind === 'transport';
        },
      }),
      wait: this.options.wait,
      visible: this.options.visible,
      jitter: this.options.jitter,
    });
    this.#owner = owner;
    void owner
      .reconnect()
      .catch(() => 'ended' as const)
      .then((outcome) => {
        // A stopped series (reset, dispose, reopen, ending) already cleared itself.
        if (this.#owner !== owner) return;
        this.#owner = null;
        const state = this.#state;
        if (state.phase !== 'reconnecting' || state.kind !== 'transport') return;
        // Ended without connecting; or connected while a fresh failure arrived, which needs a new series.
        if (outcome === 'ended' || !this.#canReconnect()) this.#end(state.error);
        else this.#loop();
      });
  }
}
