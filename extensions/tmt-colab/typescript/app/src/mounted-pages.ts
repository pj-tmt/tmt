import type { PageBinding, PageSnapshot, PageLease } from './transport.js';

/** Live remains the only connection, writer and admitted-view owner for a mounted page. */
export interface MountedPage extends PageBinding {
  readonly closed: boolean;
  readonly registration?: unknown;
  readonly epoch?: string;
  close(reason?: Error): void;
  foreground(active: boolean): void;
  snapshot(): Promise<PageSnapshot>;
  closeAndJoin(): Promise<void>;
}
type Purpose = 'page' | 'chapter';
type Consumer = { purpose: Purpose };
interface Entry {
  controller: AbortController;
  consumers: Set<Consumer>;
  ready: Promise<MountedPage>;
  live?: MountedPage;
  unhold?: () => void;
  retiring?: Promise<void>;
}

/** Mounted transport owns acquisition and release. A chapter and foreground route borrow the
 * same Live; no second writer or lifetime is created when a chapter opens on the page. */
export class MountedPages {
  #entries = new Map<string, Entry>();
  #serial = Promise.resolve();
  #cleanupFailure: unknown;
  constructor(
    private create: (id: string, signal: AbortSignal) => Promise<MountedPage>,
    private current: (page: MountedPage) => boolean = () => true,
  ) {}

  #retire(entry: Entry, reason?: Error): Promise<void> {
    if (entry.retiring) return entry.retiring;
    const cleanup = this.#serial
      .then(async () => {
        await entry.ready.catch(() => undefined);
        entry.unhold?.();
        await entry.live?.closeAndJoin();
      })
      .catch((error: unknown) => {
        this.#cleanupFailure = error;
        throw error;
      });
    entry.retiring = cleanup;
    this.#serial = cleanup.catch(() => {});
    if (reason) entry.live?.close(reason);
    entry.controller.abort();
    return cleanup;
  }

  async acquire(
    id: string,
    purpose: Purpose,
    signal?: AbortSignal,
    admitted?: {
      create(signal: AbortSignal): Promise<MountedPage>;
      current(page: MountedPage): boolean;
    },
  ): Promise<PageLease> {
    signal?.throwIfAborted();
    if (this.#cleanupFailure) throw this.#cleanupFailure;
    const current = admitted?.current ?? this.current;
    let entry = this.#entries.get(id);
    const replacingForeground =
      purpose === 'page' &&
      !!entry &&
      ![...entry.consumers].some((consumer) => consumer.purpose === 'chapter');
    if (
      entry &&
      (replacingForeground || (entry.live && (entry.live.closed || !current(entry.live))))
    ) {
      if (this.#entries.get(id) === entry) this.#entries.delete(id);
      await this.#retire(
        entry,
        replacingForeground ? undefined : new Error('Remote session replaced'),
      );
      signal?.throwIfAborted();
      entry = this.#entries.get(id);
    }
    if (!entry) {
      const next: Entry = {
        controller: new AbortController(),
        consumers: new Set(),
        ready: undefined!,
      };
      entry = next;
      this.#entries.set(id, next);
      next.ready = this.#serial.then(async () => {
        if (this.#cleanupFailure) throw this.#cleanupFailure;
        next.controller.signal.throwIfAborted();
        const live = await (admitted
          ? admitted.create(next.controller.signal)
          : this.create(id, next.controller.signal));
        next.live = live;
        next.controller.signal.throwIfAborted();
        if (!current(live)) throw new Error('Remote session replaced');
        live.foreground([...next.consumers].some((consumer) => consumer.purpose === 'page'));
        await live.snapshot();
        // Hold the Live during route effect replay and acquisition handoff. Only this owner
        // releases that hold, after the last borrowing consumer has retired.
        next.unhold = live.subscribe(
          () => {},
          () => {},
        );
        return live;
      });
      this.#serial = next.ready.then(
        () => {},
        () => {},
      );
    }
    const owned = entry;
    const consumer = { purpose };
    owned.consumers.add(consumer);
    owned.live?.foreground([...owned.consumers].some((value) => value.purpose === 'page'));
    const subscriptions = new Set<() => void>();
    let released: Promise<void> | undefined;
    let revision = 0;
    const release = (): Promise<void> => {
      if (released) return released;
      signal?.removeEventListener('abort', abort);
      for (const unsubscribe of subscriptions) unsubscribe();
      subscriptions.clear();
      owned.consumers.delete(consumer);
      owned.live?.foreground([...owned.consumers].some((value) => value.purpose === 'page'));
      if (owned.consumers.size) return (released = Promise.resolve());
      if (this.#entries.get(id) === owned) this.#entries.delete(id);
      return (released = this.#retire(owned));
    };
    const abort = () => {
      void release().catch(() => {});
    };
    signal?.addEventListener('abort', abort, { once: true });
    try {
      const live = await owned.ready;
      signal?.throwIfAborted();
      if (released || !current(live)) throw new Error('Page released');
      const snapshot = await live.snapshot();
      signal?.throwIfAborted();
      if (!current(live) || owned.controller.signal.aborted) throw new Error('Page released');
      const binding: PageBinding = {
        get ask() {
          return live.ask;
        },
        get discussion() {
          return live.discussion;
        },
        get status() {
          return live.status;
        },
        get files() {
          return live.files;
        },
        markThreadStatusSeen: live.markThreadStatusSeen?.bind(live),
        reconnect: live.reconnect?.bind(live),
        edit: live.edit.bind(live),
        usage: live.usage?.bind(live),
        compact: live.compact?.bind(live),
        export: live.export.bind(live),
        close: abort,
        subscribe: (publish, failed) => {
          if (released || owned.controller.signal.aborted) {
            failed(new Error('Page released'));
            return () => {};
          }
          revision++;
          const unsubscribe = live.subscribe(publish, failed);
          subscriptions.add(unsubscribe);
          return () => {
            unsubscribe();
            subscriptions.delete(unsubscribe);
            const stopped = ++revision;
            queueMicrotask(() => {
              if (!subscriptions.size && stopped === revision) abort();
            });
          };
        },
      };
      return { snapshot: { ...snapshot, binding }, release };
    } catch (error) {
      await release();
      throw error;
    }
  }
}
