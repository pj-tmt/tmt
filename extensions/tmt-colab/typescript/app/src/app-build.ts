import { useSyncExternalStore } from 'react';

const ENTRY_SCRIPTS = /<script\b[^>]*\btype="module"[^>]*\bsrc="([^"]+)"/gi;
/** One served-build read per this long; failures never poll. */
const CHECK_INTERVAL_MS = 60_000;
const READ_LIMIT = 64 * 1024;

/** The app build is the content-hashed entry module a page references; a page may also
 * carry tooling modules (the dev server's client), so the hashed bundle wins. */
function pick(paths: string[]): string | null {
  return paths.find((path) => stamp(path)) ?? paths[0] ?? null;
}

export function entryOf(html: string, base: URL): string | null {
  const paths: string[] = [];
  for (const match of html.slice(0, READ_LIMIT).matchAll(ENTRY_SCRIPTS)) {
    try {
      paths.push(new URL(match[1], base).pathname);
    } catch {
      // A malformed reference is not an entry.
    }
  }
  return pick(paths);
}

/** `/assets/index-AbC123xY.js` is build `index` at hash `AbC123xY`; unhashed entries
 * (the guidance page's `recovery.js`, the reader's `reader.js`) are not app builds. */
function stamp(path: string): { name: string; hash: string } | null {
  const match = /([^/]+)-([A-Za-z0-9_-]{8})\.js$/.exec(path);
  return match ? { name: match[1], hash: match[2] } : null;
}

/** A real upgrade: the same bundle name at a different hash. A different kind of
 * page (such as pairing guidance served to an unpaired browser) says nothing. */
export function isNewerBuild(loaded: string, served: string): boolean {
  const have = stamp(loaded);
  const now = stamp(served);
  return !!have && !!now && have.name === now.name && have.hash !== now.hash;
}

/** The build this tab runs, or null when the page was not loaded from a module entry. */
export function loadedEntry(root: Document = document): string | null {
  const scripts = root.querySelectorAll<HTMLScriptElement>('script[type="module"][src]');
  return pick([...scripts].map((script) => new URL(script.src, root.baseURI).pathname));
}

/** The build the server serves now: one uncached read of this app's root page. */
async function servedEntry(): Promise<string | null> {
  const root = new URL(location.href);
  root.hash = '';
  root.search = '';
  const response = await fetch(root, {
    cache: 'no-store',
    credentials: 'same-origin',
    signal: AbortSignal.timeout(5000),
  });
  return response.ok ? entryOf(await response.text(), root) : null;
}

export interface BuildSources {
  loaded(): string | null;
  served(): Promise<string | null>;
  now(): number;
}

/**
 * Whether this tab runs an older app build than the server now serves. Stale only on a
 * real hash change of the same bundle: an offline read or a different page says nothing.
 * Callers ask on failure paths and navigation; there is no polling.
 */
export function createBuildWatch(
  sources: BuildSources = {
    loaded: loadedEntry,
    served: servedEntry,
    now: () => performance.now(),
  },
) {
  let stale = false;
  let checking = false;
  let last = -Infinity;
  const listeners = new Set<() => void>();
  return {
    stale: () => stale,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => void listeners.delete(listener);
    },
    async check(): Promise<boolean> {
      if (stale || checking || sources.now() - last < CHECK_INTERVAL_MS) return stale;
      checking = true;
      last = sources.now();
      try {
        const loaded = sources.loaded();
        const served = loaded ? await sources.served() : null;
        if (loaded && served && isNewerBuild(loaded, served)) {
          stale = true;
          listeners.forEach((listener) => listener());
        }
      } catch {
        // An unreachable server is not an upgrade.
      } finally {
        checking = false;
      }
      return stale;
    },
  };
}

export const buildWatch = createBuildWatch();

export function useStaleBuild(watch = buildWatch): boolean {
  return useSyncExternalStore(watch.subscribe, watch.stale);
}
