/**
 * The one place that decides when a broken connection is tried again (#2572). A page whose
 * transport or Remote session ended while the device is still paired is reconnected here, silently,
 * with growing delays: it never waits for a click. What counts as ended stays with the caller's
 * attempt, so a later classifier change moves no retry logic.
 */
export type AttemptOutcome = 'connected' | 'retry' | 'ended';

const DELAYS_MS = [1_000, 2_000, 4_000, 8_000];
const STEADY_MS = 15_000;

/** The wait before attempt `index` (0-based), spread by `jitter()` in [0, 1). */
export function backoffMs(index: number, jitter: () => number = Math.random): number {
  const base = DELAYS_MS[index] ?? STEADY_MS;
  return Math.round(base * (0.8 + 0.4 * jitter()));
}

/**
 * One reconnect attempt's outcome for a page whose failure state lives with the caller: a
 * successful reconnect clears it, and a failed or rejected one keeps the series going only while
 * the page's current failure is still recoverable (a rejection leaves the failure unchanged).
 */
export function reconnectAttempt(page: {
  reconnect(): Promise<boolean>;
  /** The page's current failure, or null when it is connected. */
  failure(): Error | null;
  recovered(interrupted: Error | null): void;
  recoverable(failure: Error | null): boolean;
}): () => Promise<AttemptOutcome> {
  return async () => {
    const interrupted = page.failure();
    try {
      if (await page.reconnect()) {
        page.recovered(interrupted);
        return 'connected';
      }
    } catch {
      return page.recoverable(page.failure()) ? 'retry' : 'ended';
    }
    // A fresh interruption is still recoverable; no new failure, or a terminal one, ends it.
    const failure = page.failure();
    return failure !== interrupted && page.recoverable(failure) ? 'retry' : 'ended';
  };
}

export interface ConnectionOwnerOptions {
  /** One reconnect try: `retry` means the failure is still recoverable. */
  attempt(): Promise<AttemptOutcome>;
  /** Resolves after `ms`, or as soon as `signal` aborts. */
  wait?(ms: number, signal: AbortSignal): Promise<void>;
  /** Resolves when the page is visible again; a hidden tab makes no attempts. */
  visible?(signal: AbortSignal): Promise<void>;
  jitter?(): number;
}

const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve) => {
    if (signal.aborted) return resolve();
    const timer = setTimeout(done, ms);
    function done() {
      clearTimeout(timer);
      signal.removeEventListener('abort', done);
      resolve();
    }
    signal.addEventListener('abort', done, { once: true });
  });

const untilVisible = (signal: AbortSignal) =>
  new Promise<void>((resolve) => {
    if (signal.aborted || typeof document === 'undefined' || document.visibilityState !== 'hidden')
      return resolve();
    const done = () => {
      document.removeEventListener('visibilitychange', check);
      signal.removeEventListener('abort', done);
      resolve();
    };
    const check = () => document.visibilityState !== 'hidden' && done();
    document.addEventListener('visibilitychange', check);
    signal.addEventListener('abort', done, { once: true });
  });

export class ConnectionOwner {
  #controller: AbortController | null = null;
  constructor(private readonly options: ConnectionOwnerOptions) {}
  get reconnecting(): boolean {
    return this.#controller !== null;
  }
  /** Retry until connected or ended. A call while a series runs is ignored (`stopped`): one loop only. */
  async reconnect(): Promise<AttemptOutcome | 'stopped'> {
    if (this.#controller) return 'stopped';
    const controller = (this.#controller = new AbortController());
    const { signal } = controller;
    const wait = this.options.wait ?? sleep;
    const visible = this.options.visible ?? untilVisible;
    try {
      for (let index = 0; ; index++) {
        await wait(backoffMs(index, this.options.jitter), signal);
        await visible(signal);
        if (signal.aborted) return 'stopped';
        const outcome = await this.options.attempt();
        if (signal.aborted) return 'stopped';
        if (outcome !== 'retry') return outcome;
      }
    } finally {
      if (this.#controller === controller) this.#controller = null;
    }
  }
  /** Cancel the series: no further attempt starts and no late outcome is reported. */
  stop(): void {
    this.#controller?.abort();
  }
}
