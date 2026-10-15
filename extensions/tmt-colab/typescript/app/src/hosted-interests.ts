import type {
  SealedSyncPort,
  SyncCall,
  SyncInterest,
  SyncFailure,
  SyncSubscription,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';

const NEXT_MS = 30_000;
interface Group {
  controller: AbortController;
  signal: AbortSignal;
  subscriptions: SyncSubscription[];
  watchers: Promise<void>[];
  refresh: (call: SyncCall) => Promise<void>;
  refreshing?: Promise<void>;
  joining?: Promise<void>;
  dirty: boolean;
  failure?: SyncFailure;
  initialDeadlineAtMs?: number;
  aborted: () => void;
}

/** One active page group; the caller owns verification and query positions. */
export class HostedInterests {
  private generation = 0;
  private changes = Promise.resolve();
  private group?: Group;
  constructor(
    private readonly port: SealedSyncPort,
    private readonly drain: () => Promise<void>,
    private readonly failed: (failure: SyncFailure) => void,
  ) {}
  replace(
    interests: readonly SyncInterest[],
    refresh: (call: SyncCall) => Promise<void>,
    call: SyncCall,
  ): Promise<void> {
    if (interests.length < 1 || interests.length > 3)
      return Promise.reject({ code: 'REMOTE_RATE_LIMITED' });
    const selected = structuredClone(interests);
    const generation = ++this.generation;
    if (this.group) this.fence(this.group);
    const change = this.changes.then(async () => {
      await this.stop();
      if (generation !== this.generation || call.signal.aborted) throw cancelled();
      if (!Number.isFinite(call.deadlineAtMs) || Date.now() >= call.deadlineAtMs) throw deadline();
      const controller = new AbortController();
      const group: Group = {
        controller,
        signal: AbortSignal.any([controller.signal, call.signal]),
        subscriptions: [],
        watchers: [],
        refresh,
        dirty: false,
        initialDeadlineAtMs: call.deadlineAtMs,
        aborted: () => this.fence(group),
      };
      this.group = group;
      group.signal.addEventListener('abort', group.aborted, { once: true });
      try {
        for (const interest of selected) {
          const subscription = await this.port.subscribe(interest, {
            ...call,
            signal: group.signal,
          });
          if (group.signal.aborted) {
            subscription.close();
            throw cancelled();
          }
          group.subscriptions.push(subscription);
          group.watchers.push(this.watch(group, subscription));
        }
        this.dirty(group);
        await group.refreshing;
        if (group.failure) throw group.failure;
        if (group.signal.aborted) throw cancelled();
        group.initialDeadlineAtMs = undefined;
      } catch (error) {
        if (!group.signal.aborted) this.end(group, error);
        await this.stop();
        throw group.failure ?? error;
      }
    });
    this.changes = change.then(
      () => {},
      () => {},
    );
    return change;
  }
  close(): Promise<void> {
    ++this.generation;
    if (this.group) this.fence(this.group);
    const close = this.changes.then(() => this.stop());
    this.changes = close.then(
      () => {},
      () => {},
    );
    return close;
  }
  private async watch(group: Group, subscription: SyncSubscription): Promise<void> {
    while (!group.signal.aborted) {
      const call = { signal: group.signal, deadlineAtMs: Date.now() + NEXT_MS };
      try {
        await subscription.next(call);
        if (!group.signal.aborted) this.dirty(group);
      } catch (error) {
        if (group.signal.aborted) return;
        // Only an elapsed idle waiter may continue on the same interest. An early
        // claimed deadline is a broken port, not permission to run a hot await loop.
        if (isDeadline(error) && Date.now() >= call.deadlineAtMs) continue;
        this.end(group, error);
        return;
      }
    }
  }
  private dirty(group: Group): void {
    if (group.signal.aborted) return;
    group.dirty = true;
    if (group.refreshing) return;
    const work = (async () => {
      while (group.dirty && !group.signal.aborted) {
        group.dirty = false;
        await group.refresh({
          signal: group.signal,
          deadlineAtMs: Math.min(group.initialDeadlineAtMs ?? Infinity, Date.now() + NEXT_MS),
        });
      }
    })();
    group.refreshing = work;
    const complete = () => {
      group.refreshing = undefined;
      if (group.dirty && !group.signal.aborted) this.dirty(group);
    };
    void work.then(complete, (error) => {
      if (!group.signal.aborted) this.end(group, error);
      complete();
    });
  }
  private end(group: Group, error: unknown): void {
    if (group.failure || group.signal.aborted) return;
    group.failure = refusal(error);
    this.fence(group);
    this.failed(group.failure);
  }
  private fence(group: Group): void {
    group.controller.abort();
    group.subscriptions.forEach((subscription) => subscription.close());
  }
  private async stop(): Promise<void> {
    const group = this.group;
    if (!group) return;
    this.fence(group);
    group.joining ??= (async () => {
      await Promise.allSettled([
        ...group.watchers,
        ...(group.refreshing ? [group.refreshing] : []),
      ]);
      await this.drain();
      group.signal.removeEventListener('abort', group.aborted);
    })();
    await group.joining;
    if (this.group === group) this.group = undefined;
  }
}

const cancelled = (): SyncFailure => ({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
const deadline = (): SyncFailure => ({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'deadline' });
function isDeadline(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    error.code === 'REMOTE_STATE_UNAVAILABLE' &&
    'state' in error &&
    error.state === 'deadline'
  );
}
function refusal(error: unknown): SyncFailure {
  if (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    [
      'REMOTE_INPUT_INVALID',
      'REMOTE_SCOPE_DENIED',
      'REMOTE_STATE_UNAVAILABLE',
      'REMOTE_RATE_LIMITED',
      'REMOTE_BUDGET_EXHAUSTED',
    ].includes(String(error.code))
  )
    return error as SyncFailure;
  return { code: 'REMOTE_STATE_UNAVAILABLE' };
}
