import { useEffect, useMemo, useSyncExternalStore } from 'react';
import { readRecentlyOpened } from './recently-opened.js';
import type { PageSummary, PageTransport } from './transport.js';

/** What a quick switcher can show: nothing while closed, then the page list and the device's
 * recently opened pages. `failed` only when the list could not be read and there is nothing to show. */
export type SwitcherSource =
  | { state: 'idle' }
  | { state: 'loading' }
  | { state: 'ready'; pages: readonly PageSummary[]; opened: ReadonlyMap<string, number> }
  | { state: 'failed' };

const IDLE: SwitcherSource = { state: 'idle' };
const LOADING: SwitcherSource = { state: 'loading' };
const FAILED: SwitcherSource = { state: 'failed' };
type Cached = Extract<SwitcherSource, { state: 'ready' }>;

/** The last list read per transport and space, so the next opening shows it at once. */
const cache = new WeakMap<PageTransport, Map<string | undefined, Cached>>();

/**
 * One switcher's read of the owner list, for one transport and space. `open()` starts a read and
 * `close()` ends it: a result that arrives after a close, a reopen or a new store is dropped. A
 * cached list is shown immediately and refreshed in the background; a refresh that fails keeps it.
 */
export class SwitcherSourceStore {
  #snapshot: SwitcherSource = IDLE;
  #generation = 0;
  #listeners = new Set<() => void>();
  constructor(
    private readonly transport: PageTransport,
    private readonly space: string | undefined,
    private readonly readOpened: typeof readRecentlyOpened = readRecentlyOpened,
  ) {}
  subscribe = (listener: () => void) => {
    this.#listeners.add(listener);
    return () => void this.#listeners.delete(listener);
  };
  getSnapshot = () => this.#snapshot;
  /** What the first render of an open switcher shows: the cached list, else loading. The same
   * object until the cache changes. */
  initial(): SwitcherSource {
    return cache.get(this.transport)?.get(this.space) ?? LOADING;
  }
  #set(next: SwitcherSource) {
    if (next === this.#snapshot) return;
    this.#snapshot = next;
    for (const listener of this.#listeners) listener();
  }
  open() {
    const generation = ++this.#generation;
    this.#set(this.initial());
    void (async () => {
      const [list, opened] = await Promise.allSettled([
        this.transport.spaceHome(),
        this.readOpened(this.space),
      ]);
      if (generation !== this.#generation) return;
      if (list.status === 'fulfilled') {
        const ready: Cached = {
          state: 'ready',
          pages: list.value.pages,
          opened: opened.status === 'fulfilled' ? opened.value : new Map(),
        };
        let spaces = cache.get(this.transport);
        if (!spaces) cache.set(this.transport, (spaces = new Map()));
        spaces.set(this.space, ready);
        this.#set(ready);
      } else if (this.#snapshot.state !== 'ready') this.#set(FAILED);
    })();
  }
  close() {
    this.#generation++;
    this.#set(IDLE);
  }
}

/**
 * The data a quick switcher ranks. While `open` is false it is `idle` and reads nothing. Opening
 * reads the page list and the recently opened pages again; the first render after opening already
 * shows the cached list when there is one. A change of `transport` or `space` starts a new read.
 */
export function useSwitcherSource(
  open: boolean,
  transport: PageTransport,
  space: string | undefined,
): SwitcherSource {
  const store = useMemo(() => new SwitcherSourceStore(transport, space), [transport, space]);
  useEffect(() => {
    if (!open) return;
    store.open();
    return () => store.close();
  }, [open, store]);
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  if (!open) return IDLE;
  return snapshot.state === 'idle' ? store.initial() : snapshot;
}
