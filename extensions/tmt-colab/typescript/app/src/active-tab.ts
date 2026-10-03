/** Local tab ownership is UI coordination, never Remote or page authority. */
export class InactiveTabError extends Error {
  constructor() {
    super('Colab is open in another tab.');
  }
}
interface Claim {
  type: 'takeover';
  tab: string;
  at: number;
}
export interface TabOwnership {
  readonly active: boolean;
  run<T>(action: () => Promise<T>): Promise<T>;
}
function newer(a: Claim, b: Claim | null) {
  return !b || a.at > b.at || (a.at === b.at && a.tab > b.tab);
}

/** A takeover announces before waiting for the lease. The old tab stops local
 * work immediately, then releases only after already-started Remote calls end.
 * Holding the Web Lock prevents a new session racing those calls. Stale claims
 * cannot reactivate a tab; only an explicit new takeover can acquire the lease. */
export class ActiveTab implements TabOwnership {
  #channel: BroadcastChannel;
  #tab = crypto.randomUUID();
  #claim: Claim | null = null;
  #active = false;
  #closed = false;
  #abort: AbortController | null = null;
  #release: (() => void) | null = null;
  #running = 0;
  #pending: Promise<void> | null = null;
  constructor(
    private key: string,
    private inactive: () => void,
  ) {
    this.#channel = new BroadcastChannel(`colab-tab:${key}`);
    this.#channel.onmessage = ({ data }) => {
      if (
        !data ||
        Object.keys(data).length !== 3 ||
        data.type !== 'takeover' ||
        typeof data.tab !== 'string' ||
        data.tab.length !== 36 ||
        typeof data.at !== 'number' ||
        !Number.isFinite(data.at) ||
        !newer(data, this.#claim)
      )
        return;
      this.#claim = data;
      this.#stop();
    };
  }
  get active() {
    return this.#active && !this.#closed;
  }
  #stop() {
    const wasRunning = this.#active || this.#pending !== null;
    this.#active = false;
    this.#abort?.abort();
    if (wasRunning) this.inactive();
    if (this.#running === 0) this.#release?.();
  }
  takeover(): Promise<void> {
    if (this.#closed) return Promise.reject(new InactiveTabError());
    if (this.active) return Promise.resolve();
    if (this.#pending) return this.#pending;
    const claim: Claim = {
      type: 'takeover',
      tab: this.#tab,
      at: Math.max(performance.timeOrigin + performance.now(), (this.#claim?.at ?? 0) + 1),
    };
    this.#claim = claim;
    const abort = (this.#abort = new AbortController());
    let resolve!: () => void, reject!: (error: unknown) => void;
    const ready = new Promise<void>((yes, no) => {
      resolve = yes;
      reject = no;
    });
    // This precedes both acquiring the lease and every reopenSession invocation.
    this.#channel.postMessage(claim);
    void navigator.locks
      .request(`colab-tab:${this.key}`, { signal: abort.signal }, async () => {
        if (this.#closed || this.#claim !== claim) throw new InactiveTabError();
        await new Promise<void>((release) => {
          this.#release = release;
          this.#active = true;
          resolve();
        });
        this.#release = null;
      })
      .catch(reject);
    const pending = ready.finally(() => {
      if (this.#pending === pending) this.#pending = null;
    });
    this.#pending = pending;
    return pending;
  }
  async run<T>(action: () => Promise<T>): Promise<T> {
    if (!this.active) throw new InactiveTabError();
    this.#running++;
    try {
      return await action();
    } finally {
      this.#running--;
      if (!this.active && this.#running === 0) this.#release?.();
    }
  }
  close() {
    if (this.#closed) return;
    this.#closed = true;
    this.#stop();
    this.#channel.close();
  }
}
