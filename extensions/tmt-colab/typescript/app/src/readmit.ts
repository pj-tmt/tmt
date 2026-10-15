import { classify } from './connection-state.js';

/** At most `limit` automatic reopens inside `windowMs`; a page that keeps failing then stops. */
export class ReadmitBudget {
  #times: number[] = [];
  constructor(
    private readonly limit = 3,
    private readonly windowMs = 60_000,
  ) {}
  take(now: number): boolean {
    this.#times = this.#times.filter((time) => now - time < this.windowMs);
    if (this.#times.length >= this.limit) return false;
    this.#times.push(now);
    return true;
  }
}

/** Pause before each reopen attempt: the first is immediate, later ones wait out a lagging epoch. */
export const READMIT_DELAYS_MS: readonly number[] = [0, 1000, 3000, 8000, 15_000];

/** A failed reopen worth another attempt: a readmittable refusal or a cause nobody classified. */
export function retryableReopen(error: Error): boolean {
  const { recovery, cause } = classify(error);
  return recovery === 'reopen' || cause === 'generic';
}

export type Reopened<T> = { readonly value: T } | { readonly error: Error };

/**
 * Reopen a page for its new epoch, retrying a lagging epoch or a transient refusal with the
 * delays above. A refusal that says the access ended, or the last failed attempt, is the answer.
 */
export async function reopenWithBackoff<T>(
  open: () => Promise<T>,
  options: {
    signal: AbortSignal;
    retryable: (error: Error) => boolean;
    wait?: (ms: number, signal: AbortSignal) => Promise<void>;
    delays?: readonly number[];
  },
): Promise<Reopened<T>> {
  const delays = options.delays ?? READMIT_DELAYS_MS;
  const wait = options.wait ?? waitReadmitDelay;
  let last = new Error('Reopen aborted');
  for (const delay of delays) {
    if (delay > 0) await wait(delay, options.signal);
    if (options.signal.aborted) return { error: last };
    try {
      return { value: await open() };
    } catch (error) {
      last = error instanceof Error ? error : new Error(String(error));
      if (!options.retryable(last)) return { error: last };
    }
  }
  return { error: last };
}

/** Shared bounded reopen delay; cancellation wakes it and removes its listener. */
export function waitReadmitDelay(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) return resolve();
    const done = () => {
      clearTimeout(timer);
      signal.removeEventListener('abort', done);
      resolve();
    };
    const timer = setTimeout(done, ms);
    signal.addEventListener('abort', done, { once: true });
  });
}

/**
 * One page's reopen state: at most one attempt in flight, and the controller of the binding the
 * last successful reopen produced (its signal closes that binding). `reset()` ends both, so a
 * route reload that brings a newer binding cannot leave a reopened one open beside it.
 */
export class PageReopener<T> {
  #pending: AbortController | null = null;
  #life: AbortController | null = null;

  get busy(): boolean {
    return this.#pending !== null;
  }

  start(
    open: (signal: AbortSignal) => Promise<T>,
    options: {
      retryable: (error: Error) => boolean;
      done: (result: Reopened<T>) => void;
      wait?: (ms: number, signal: AbortSignal) => Promise<void>;
    },
  ): void {
    if (this.#pending) return;
    const controller = new AbortController();
    this.#pending = controller;
    void reopenWithBackoff(() => open(controller.signal), {
      signal: controller.signal,
      retryable: options.retryable,
      wait: options.wait,
    }).then((result) => {
      if (controller.signal.aborted) return;
      this.#pending = null;
      if ('value' in result) {
        this.#life?.abort();
        this.#life = controller;
      }
      options.done(result);
    });
  }

  reset(): void {
    this.#pending?.abort();
    this.#life?.abort();
    this.#pending = null;
    this.#life = null;
  }
}
