import type {
  SyncAdmissionCost,
  SyncAdmissionGuard,
} from '../../../../tmt-remote/typescript/remote-client/src/firebase-sync-http.js';
import type * as budget from '../../../../tmt-remote/typescript/remote-client/src/budget.js';
import { record } from './storage.js';

export interface HostedUsage {
  day: number;
  reads: number;
  writes: number;
}
/** The origin owner serializes a complete read/modify/write across tabs. */
export interface HostedBudgetStore {
  transact<T>(change: (stored: unknown) => { usage: HostedUsage; result: T }): Promise<T>;
}
export type HostedBudgetMath = Pick<
  typeof budget,
  'READS_PER_DAY' | 'WARN_PERCENT' | 'REFUSE_PERCENT' | 'pacificDay'
>;

/** Hosted link reads share ten percent of the free daily reads on this browser origin. */
export class HostedBudget implements SyncAdmissionGuard {
  private readonly pending = new Set<Promise<unknown>>();
  constructor(
    private readonly math: HostedBudgetMath,
    private readonly store: HostedBudgetStore,
    private readonly now: () => number = Date.now,
  ) {}
  admit(cost: SyncAdmissionCost): Promise<void> {
    return this.track(async () => {
      const { operation, reads, writes, rulesLookupsPerDocument: lookups } = cost;
      if (operation === 'listen') {
        if (reads !== null || writes !== 0) throw unavailable();
        await this.charge(0, 0, false);
      } else if (operation === 'commit') {
        // The sole hosted mutation is one members create, with four admission lookups.
        if (reads !== 0 || writes !== 1 || lookups !== 4) throw unavailable();
        await this.charge(lookups, 1, false);
      } else {
        if (
          (operation !== 'read' && operation !== 'query') ||
          !whole(reads) ||
          writes !== 0 ||
          lookups !== 3
        )
          throw unavailable();
        await this.charge(Math.max(reads, 1) * (1 + lookups), 0, false);
      }
    });
  }
  received(documents: number): Promise<void> {
    return this.track(async () => {
      if (!whole(documents)) throw unavailable();
      // A batch already delivered must remain counted even when it crosses the refusal level.
      await this.charge(documents, 0, true);
    });
  }
  snapshot(): Promise<HostedUsage & { state: 'ok' | 'warn' | 'refuse' }> {
    return this.track(() =>
      this.store.transact((stored) => {
        const usage = this.today(stored, this.now());
        return { usage, result: { ...usage, state: this.state(usage.reads) } };
      }),
    );
  }
  /** Called after all interests are fenced/closed, before a successor page is opened. */
  async drain(): Promise<void> {
    await Promise.allSettled(this.pending);
  }
  private state(reads: number): 'ok' | 'warn' | 'refuse' {
    const allowance = Math.floor(this.math.READS_PER_DAY / 10);
    if (reads >= Math.floor((allowance * this.math.REFUSE_PERCENT) / 100)) return 'refuse';
    if (reads >= Math.floor((allowance * this.math.WARN_PERCENT) / 100)) return 'warn';
    return 'ok';
  }
  private today(stored: unknown, now: number): HostedUsage {
    const { day } = this.math.pacificDay(now);
    if (!Number.isSafeInteger(day)) throw unavailable();
    if (stored === undefined) return { day, reads: 0, writes: 0 };
    if (
      typeof stored !== 'object' ||
      stored === null ||
      Array.isArray(stored) ||
      Object.keys(stored).length !== 3 ||
      !('day' in stored) ||
      !('reads' in stored) ||
      !('writes' in stored) ||
      !Number.isSafeInteger(stored.day) ||
      !whole(stored.reads) ||
      !whole(stored.writes) ||
      (stored.day as number) > day
    )
      throw unavailable();
    return stored.day === day
      ? { day, reads: stored.reads, writes: stored.writes }
      : { day, reads: 0, writes: 0 };
  }
  private async charge(reads: number, writes: number, delivered: boolean): Promise<void> {
    if (!whole(reads)) throw unavailable();
    const now = this.now();
    const refused = await this.store.transact((stored) => {
      const usage = this.today(stored, now);
      const next = {
        ...usage,
        reads: Math.min(Number.MAX_SAFE_INTEGER, usage.reads + reads),
        writes: Math.min(Number.MAX_SAFE_INTEGER, usage.writes + writes),
      };
      const refused = this.state(next.reads) === 'refuse';
      return { usage: refused && !delivered ? usage : next, result: refused };
    });
    if (refused) {
      const { resetAtMs } = this.math.pacificDay(now);
      throw {
        code: 'REMOTE_BUDGET_EXHAUSTED',
        resetAtMs,
        retryAfterMs: Math.max(resetAtMs - now, 0),
      };
    }
  }
  private track<T>(action: () => Promise<T>): Promise<T> {
    const job = Promise.resolve()
      .then(action)
      .catch((error: unknown) => {
        if (
          typeof error === 'object' &&
          error !== null &&
          'code' in error &&
          error.code === 'REMOTE_BUDGET_EXHAUSTED'
        )
          throw error;
        throw unavailable();
      });
    this.pending.add(job);
    void job.then(
      () => this.pending.delete(job),
      () => this.pending.delete(job),
    );
    return job;
  }
}

const unavailable = () => ({ code: 'REMOTE_STATE_UNAVAILABLE' as const });
function whole(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}
/** Web Locks serialize the durable origin record, including independent tabs. No secrets are stored. */
export function hostedBudgetStore(): HostedBudgetStore {
  return {
    async transact(change) {
      if (!navigator.locks) throw unavailable();
      return navigator.locks.request(
        'tmt-colab-hosted-reads-v1',
        { signal: AbortSignal.timeout(10_000) },
        async () => {
          const update = change(await record<unknown>('hosted-reads-v1'));
          await record('hosted-reads-v1', update.usage);
          return update.result;
        },
      );
    },
  };
}
