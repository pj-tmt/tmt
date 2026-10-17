import { MAX_RENDER_SOURCE_BYTES } from './renderer.js';
import type { PageLease, PageSnapshot } from './transport.js';

export const CHAPTER_SURFACE_LIMIT = 3;
export type ChapterPassageView = { id: string; visible: boolean } & (
  | { state: 'loading' | 'unavailable' }
  | { state: 'ready'; snapshot: PageSnapshot }
);
interface Entry {
  id: string;
  controller: AbortController;
  view: ChapterPassageView;
  lease?: PageLease;
  working: Set<string>;
  idle: Set<() => void>;
  retireFrame?: () => void;
}

/** One visible-chapter acquisition/fold queue. Parent actions already begun keep their admitted
 * binding until they settle, even after the frame retires. A successor always joins that release. */
export class ChapterPassages {
  #entries = new Map<string, Entry>();
  #wanted: string[] = [];
  #view: readonly ChapterPassageView[] = [];
  #listeners = new Set<() => void>();
  #work?: Promise<void>;
  #dirty = false;
  #stopped = false;
  #cleanupFailure: unknown;
  constructor(private acquire: (id: string, signal: AbortSignal) => Promise<PageLease>) {}

  get snapshot() {
    return this.#view;
  }
  subscribe = (changed: () => void) => {
    this.#listeners.add(changed);
    return () => {
      this.#listeners.delete(changed);
    };
  };
  #visible(id: string) {
    return !this.#stopped && !this.#cleanupFailure && this.#wanted.includes(id);
  }
  #publish() {
    this.#view = [...this.#entries.values()].map((entry) => ({
      ...entry.view,
      visible: this.#visible(entry.id),
    }));
    if (this.#cleanupFailure && !this.#stopped) {
      for (const id of this.#wanted)
        if (!this.#entries.has(id))
          this.#view = [...this.#view, { id, visible: true, state: 'unavailable' }];
    }
    this.#listeners.forEach((changed) => changed());
  }
  update(order: readonly string[], intersecting: ReadonlySet<string>, shown = true) {
    this.#wanted =
      !this.#stopped && shown
        ? [...new Set(order)].filter((id) => intersecting.has(id)).slice(0, CHAPTER_SURFACE_LIMIT)
        : [];
    for (const entry of this.#entries.values()) {
      if (this.#visible(entry.id)) continue;
      entry.retireFrame?.();
      if (!entry.working.size) entry.controller.abort();
    }
    this.#publish();
    this.#request();
  }
  /** Called by the existing action owners, synchronously before and after their work. */
  working(id: string, action: string, active: boolean) {
    const entry = this.#entries.get(id);
    if (!entry) return;
    if (active) entry.working.add(action);
    else entry.working.delete(action);
    if (!entry.working.size) {
      entry.idle.forEach((resolve) => resolve());
      entry.idle.clear();
      if (!this.#visible(id)) entry.controller.abort();
    }
    this.#request();
  }
  frame(id: string, retire: () => void) {
    const entry = this.#entries.get(id);
    if (!entry) {
      retire();
      return () => {};
    }
    entry.retireFrame = retire;
    if (!this.#visible(id)) retire();
    return () => {
      if (entry.retireFrame === retire) entry.retireFrame = undefined;
    };
  }
  #request() {
    this.#dirty = true;
    if (this.#work) return;
    this.#work = Promise.resolve()
      .then(() => this.#drain())
      .finally(() => {
        this.#work = undefined;
        if (this.#dirty) this.#request();
      });
  }
  async #release(entry: Entry) {
    entry.retireFrame?.();
    entry.controller.abort();
    const lease = entry.lease;
    entry.lease = undefined;
    try {
      await lease?.release();
    } catch (error) {
      this.#cleanupFailure = error;
    }
    if (this.#entries.get(entry.id) === entry) this.#entries.delete(entry.id);
    this.#publish();
  }
  async #drain() {
    while (this.#dirty) {
      this.#dirty = false;
      let waiting = false;
      for (const entry of this.#entries.values()) {
        if (this.#visible(entry.id)) continue;
        if (entry.working.size) {
          await new Promise<void>((resolve) => entry.idle.add(resolve));
          this.#dirty = true;
          waiting = true;
          break;
        }
        await this.#release(entry);
      }
      if (waiting) continue;
      for (const id of this.#wanted) {
        if (
          !this.#visible(id) ||
          this.#entries.has(id) ||
          this.#entries.size >= CHAPTER_SURFACE_LIMIT
        )
          continue;
        const entry: Entry = {
          id,
          controller: new AbortController(),
          working: new Set(),
          idle: new Set(),
          view: { id, visible: true, state: 'loading' },
        };
        this.#entries.set(id, entry);
        this.#publish();
        try {
          entry.lease = await this.acquire(id, entry.controller.signal);
          if (!this.#visible(id) || entry.controller.signal.aborted) {
            await this.#release(entry);
            this.#dirty = true;
            continue;
          }
          if (
            new TextEncoder().encode(entry.lease.snapshot.source).length > MAX_RENDER_SOURCE_BYTES
          ) {
            const lease = entry.lease;
            entry.lease = undefined;
            entry.controller.abort();
            try {
              await lease.release();
            } catch (error) {
              this.#cleanupFailure = error;
            }
            entry.view = { id, visible: true, state: 'unavailable' };
          } else entry.view = { id, visible: true, state: 'ready', snapshot: entry.lease.snapshot };
        } catch {
          if (entry.controller.signal.aborted || !this.#visible(id)) {
            await this.#release(entry);
            this.#dirty = true;
            continue;
          }
          entry.view = { id, visible: true, state: 'unavailable' };
        }
        this.#publish();
      }
      if (this.#cleanupFailure && this.#entries.size) this.#dirty = true;
    }
  }
  /** Event-driven join, including a replacement pass scheduled while this caller awaited. */
  async settled() {
    while (this.#work) await this.#work;
    if (this.#cleanupFailure) throw this.#cleanupFailure;
  }
  dispose(): Promise<void> {
    this.#stopped = true;
    this.update([], new Set());
    return this.settled();
  }
}
