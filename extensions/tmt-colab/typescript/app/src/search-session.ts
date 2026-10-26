/**
 * The global search's Colab half for a host that has no Colab route and builds no page transport
 * (#5267): the session owns Colab's mounted transport (admission, bootstrap, lifetime) and its
 * close, mounts the one search entry against it, and returns the entry's `open` and `dispose`.
 * The host keeps the import, the prefetch, its generation and its disposal.
 */
import type { mountedTransport } from './mounted.js';
import type { SearchEntry, SearchEntryOptions } from './search-entry.js';
import type { PageTransport } from './transport.js';

/** What the host supplies; the session adds the transport. */
export type SearchSessionOptions = Omit<SearchEntryOptions, 'transport' | 'space' | 'pageId'>;

type Admitted = Awaited<ReturnType<typeof mountedTransport>>;

export interface SearchSessionDeps {
  admit(signal: AbortSignal): Promise<Admitted>;
  mountEntry(container: HTMLElement, options: SearchEntryOptions): SearchEntry;
  /** Where a document that is going away announces `pagehide`. */
  page: Pick<EventTarget, 'addEventListener' | 'removeEventListener'>;
}

/** What an attempt has become: ready, failed, or ended (closed by the session). */
interface Track {
  state: 'starting' | 'ready' | 'failed' | 'ended';
  admitted?: Admitted;
  closed: boolean;
}
/** One try at the transport. */
interface Attempt extends Track {
  readonly controller: AbortController;
  readonly ready: Promise<Admitted>;
}

/** Waits for `work`, and no longer than `signal` lets the reader wait. */
function until<T>(signal: AbortSignal | undefined, work: Promise<T>): Promise<T> {
  if (!signal) return work;
  if (signal.aborted) return Promise.reject(signal.reason);
  return new Promise<T>((resolve, reject) => {
    const leave = () => reject(signal.reason);
    signal.addEventListener('abort', leave, { once: true });
    work.then(resolve, reject).finally(() => signal.removeEventListener('abort', leave));
  });
}

export function createSearchSession(
  deps: SearchSessionDeps,
  container: HTMLElement,
  options: SearchSessionOptions,
): SearchEntry {
  let disposed = false;
  let attempt: Attempt | undefined;

  const close = (finished: Track) => {
    if (finished.closed || !finished.admitted) return;
    finished.closed = true;
    finished.admitted.close();
  };
  const begin = (): Attempt => {
    const controller = new AbortController();
    const track: Track = { state: 'starting', closed: false };
    const ready = deps.admit(controller.signal).then(
      (admitted) => {
        track.admitted = admitted;
        // Resolved after the session ended or was disposed: nothing may keep this transport.
        if (controller.signal.aborted) {
          close(track);
          throw controller.signal.reason;
        }
        track.state = 'ready';
        return admitted;
      },
      (error) => {
        if (track.state === 'starting') track.state = 'failed';
        throw error;
      },
    );
    // Reads report the failure; nothing else is waiting on this promise.
    ready.catch(() => {});
    return Object.assign(track, { controller, ready });
  };
  const end = (finished: Attempt | undefined) => {
    if (!finished || finished.state === 'ended') return;
    finished.state = 'ended';
    finished.controller.abort(new Error('Search session closed'));
    close(finished);
  };

  // One transport object for the entry's whole life: a retry swaps the attempt behind it, so the
  // switcher never sees its transport change and close an open dialog.
  const read = <T>(
    signal: AbortSignal | undefined,
    use: (transport: PageTransport) => Promise<T>,
  ) => {
    const current = attempt;
    if (!current || current.state === 'ended')
      return Promise.reject(new Error('Search session closed'));
    return until(signal, current.ready).then((admitted) => use(admitted.transport));
  };
  const transport: PageTransport = {
    spaceHome: (signal) => read(signal, (ready) => ready.spaceHome(signal)),
    page: (id, signal) => read(signal, (ready) => ready.page(id, signal)),
  };

  attempt = begin();
  const entry = deps.mountEntry(container, { ...options, transport });
  // The tab's Remote session ends with its document, as the app's own mount does.
  const hide = () => end(attempt);
  deps.page.addEventListener('pagehide', hide);

  return {
    open(opener) {
      if (disposed) return;
      // A failed or ended attempt is tried again by the next opening, never in a loop.
      if (!attempt || attempt.state === 'failed' || attempt.state === 'ended') attempt = begin();
      entry.open(opener);
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      deps.page.removeEventListener('pagehide', hide);
      entry.dispose();
      end(attempt);
      attempt = undefined;
    },
  };
}
