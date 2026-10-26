import { expect, it, vi } from 'vite-plus/test';
import {
  createSearchSession,
  type SearchSessionDeps,
  type SearchSessionOptions,
} from '../src/search-session.js';
import type { SearchEntry, SearchEntryOptions } from '../src/search-entry.js';
import type { PageTransport } from '../src/transport.js';

type Admitted = Awaited<ReturnType<SearchSessionDeps['admit']>>;

const options = {
  registry: {} as SearchSessionOptions['registry'],
  navigate: async () => undefined,
  pageHref: (pageId: string) => `/colab/p/${pageId}`,
  go: async () => undefined,
} satisfies SearchSessionOptions;
const opener = () => ({}) as HTMLButtonElement;

/** A transport that answers `spaceHome` with the one title and counts its closes. */
function admitted(title: string) {
  const close = vi.fn();
  const value: Admitted = {
    space: 'space',
    pageIds: [],
    close,
    transport: {
      spaceHome: async () => ({ title, pages: [] }),
      page: async () => {
        throw new Error('not read here');
      },
    },
  };
  return { value, close };
}

/** The deps with every admission under the test's control. */
function harness({ honorsAbort = true } = {}) {
  const attempts: {
    signal: AbortSignal;
    resolve(value: Admitted): void;
    reject(reason: unknown): void;
  }[] = [];
  const page = new EventTarget();
  const entry = { open: vi.fn(), dispose: vi.fn() } satisfies SearchEntry;
  let seen: SearchEntryOptions | undefined;
  const deps: SearchSessionDeps = {
    admit: (signal) =>
      new Promise<Admitted>((resolve, reject) => {
        attempts.push({ signal, resolve, reject });
        // A real bootstrap that is aborted gives up with the abort's reason, unless it had
        // already finished.
        if (honorsAbort)
          signal.addEventListener('abort', () => reject(signal.reason), { once: true });
      }),
    mountEntry: (_container, given) => {
      seen = given;
      return entry;
    },
    page,
  };
  const session = createSearchSession(deps, {} as HTMLElement, options);
  const transport = (): PageTransport => seen!.transport;
  return { attempts, entry, page, session, transport, seen: () => seen! };
}

it('starts the transport when mounted and hands the entry the host options plus its own transport', () => {
  const { attempts, seen } = harness();
  expect(attempts).toHaveLength(1);
  expect(seen().registry).toBe(options.registry);
  expect(seen().pageHref('x')).toBe('/colab/p/x');
  expect(seen().transport).toBeDefined();
});

it('opens at once before the transport is ready; the pages arrive when it is, with no second open', async () => {
  const { attempts, entry, session, transport } = harness();
  const button = opener();
  session.open(button);
  expect(entry.open).toHaveBeenCalledTimes(1);
  expect(entry.open).toHaveBeenCalledWith(button);
  const read = transport().spaceHome();
  const settled = vi.fn();
  void read.then(settled);
  await Promise.resolve();
  expect(settled).not.toHaveBeenCalled();
  attempts[0]!.resolve(admitted('Studio pages').value);
  await expect(read).resolves.toEqual({ title: 'Studio pages', pages: [] });
  expect(entry.open).toHaveBeenCalledTimes(1);
  expect(attempts).toHaveLength(1);
});

it('disposing during bootstrap gives the bootstrap its abort and leaves nothing open', async () => {
  const { attempts, entry, session, transport } = harness();
  const read = transport().spaceHome();
  session.dispose();
  expect(attempts[0]!.signal.aborted).toBe(true);
  expect(entry.dispose).toHaveBeenCalledTimes(1);
  await expect(read).rejects.toThrow('Search session closed');
});

it('a reader that leaves while the transport boots stops waiting, and the boot goes on', async () => {
  const { attempts, transport } = harness();
  const reader = new AbortController();
  const read = transport().spaceHome(reader.signal);
  reader.abort(new Error('left'));
  await expect(read).rejects.toThrow('left');
  expect(attempts[0]!.signal.aborted).toBe(false);
});

it('a transport that finishes just after dispose is closed once, never kept', async () => {
  const { attempts, session, transport } = harness({ honorsAbort: false });
  const read = transport().spaceHome();
  session.dispose();
  const late = admitted('Late');
  attempts[0]!.resolve(late.value);
  await expect(read).rejects.toThrow('Search session closed');
  expect(late.close).toHaveBeenCalledTimes(1);
});

it('disposing a ready session closes its transport once, and a second dispose is inert', async () => {
  const { attempts, entry, session, transport } = harness();
  const ready = admitted('Studio pages');
  attempts[0]!.resolve(ready.value);
  await transport().spaceHome();
  session.dispose();
  session.dispose();
  expect(ready.close).toHaveBeenCalledTimes(1);
  expect(entry.dispose).toHaveBeenCalledTimes(1);
  session.open(opener());
  expect(entry.open).not.toHaveBeenCalled();
});

it('a failed bootstrap is reported to the reader and tried again by the next opening, on the same transport', async () => {
  const { attempts, entry, session, transport, seen } = harness();
  const before = seen().transport;
  attempts[0]!.reject(new Error('Remote refused'));
  await expect(transport().spaceHome()).rejects.toThrow('Remote refused');
  // The failure alone starts nothing: one try per opening.
  expect(attempts).toHaveLength(1);
  session.open(opener());
  expect(attempts).toHaveLength(2);
  expect(entry.open).toHaveBeenCalledTimes(1);
  expect(seen().transport).toBe(before);
  const read = transport().spaceHome();
  attempts[1]!.resolve(admitted('Second try').value);
  await expect(read).resolves.toEqual({ title: 'Second try', pages: [] });
  // A ready transport is not replaced by another opening.
  session.open(opener());
  expect(attempts).toHaveLength(2);
});

it('the tab leaving closes the transport; a restored page opens a new one on its next opening', async () => {
  const { attempts, page, session, transport } = harness();
  const first = admitted('First');
  attempts[0]!.resolve(first.value);
  await transport().spaceHome();
  page.dispatchEvent(new Event('pagehide'));
  expect(first.close).toHaveBeenCalledTimes(1);
  await expect(transport().spaceHome()).rejects.toThrow('Search session closed');
  session.open(opener());
  expect(attempts).toHaveLength(2);
  attempts[1]!.resolve(admitted('Restored').value);
  await expect(transport().spaceHome()).resolves.toEqual({ title: 'Restored', pages: [] });
});

it('disposing stops listening for the tab leaving', async () => {
  const { attempts, page, session } = harness();
  const ready = admitted('Studio pages');
  attempts[0]!.resolve(ready.value);
  await Promise.resolve();
  session.dispose();
  page.dispatchEvent(new Event('pagehide'));
  expect(ready.close).toHaveBeenCalledTimes(1);
});
