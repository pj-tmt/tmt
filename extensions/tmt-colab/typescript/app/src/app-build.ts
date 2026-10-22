import { entryMount, inShell, publicEntry } from './entry.js';
import { useSyncExternalStore } from 'react';
import { entryOf, isNewerBuild, pick } from './build-stamp.js';

/** One served-build read per this long; failures never poll. */
const CHECK_INTERVAL_MS = 60_000;

/** The build this tab runs, or null when the page was not loaded from a module entry. */
export function loadedEntry(root: Document = document): string | null {
  const scripts = root.querySelectorAll<HTMLScriptElement>('script[type="module"][src]');
  return pick([...scripts].map((script) => new URL(script.src, root.baseURI).pathname));
}

/** The app entry this module was built beside, written into it by `vp build --mode module` (see
 * `vite.config.ts`); not defined in the dev server or in tests. */
declare const __COLAB_MODULE_BUILD__: string | undefined;

/**
 * Inside the TMT app the document is the shell's, so the build this tab runs is the one the Colab
 * module was made with: it names itself, so nothing read after it loaded (an upgrade can land
 * between the import and `mount()`) can rename it. Unpaired (the local preview) there is none:
 * its served read is the shell's own page. With no stamp the update row stays quiet.
 */
function moduleBuild(): string | null {
  return typeof __COLAB_MODULE_BUILD__ === 'string' ? __COLAB_MODULE_BUILD__ : null;
}
function runningEntry(): string | null {
  if (!inShell()) return loadedEntry();
  return publicEntry() ? moduleBuild() : null;
}

/** The build the server serves now: one uncached read of this app's root page (inside the TMT
 * app, of its paired mount). */
async function servedEntry(): Promise<string | null> {
  const root = publicEntry() ? entryMount() : new URL(location.href);
  root.hash = '';
  root.search = '';
  const response = await fetch(root, {
    cache: 'no-store',
    credentials: 'same-origin',
    signal: AbortSignal.timeout(5000),
  });
  return response.ok ? entryOf(await response.text(), root) : null;
}

/** The serve's own release and, when it differs, the one now installed beside it. */
export interface ServeRelease {
  running: string;
  installed?: string;
  restartFailed?: true;
}
const RELEASE_VERSION = /^[0-9A-Za-z.+-]{1,64}$/;

/** Strict read of `/api/serve-release`; anything else says nothing. */
export function parseServeRelease(value: unknown): ServeRelease | null {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return null;
  const keys = Object.keys(value);
  const { running, installed, restartFailed } = value as Record<string, unknown>;
  const known = keys.every(
    (key) => key === 'running' || key === 'installed' || key === 'restartFailed',
  );
  const plain = (v: unknown): v is string => typeof v === 'string' && RELEASE_VERSION.test(v);
  if (
    !known ||
    !plain(running) ||
    (keys.includes('installed') && !plain(installed)) ||
    (keys.includes('restartFailed') && (restartFailed !== true || !plain(installed)))
  )
    return null;
  return keys.includes('installed')
    ? {
        running,
        installed: installed as string,
        ...(restartFailed === true ? { restartFailed: true as const } : {}),
      }
    : { running };
}

/** The serve's release, or null when the route is absent (an older serve) or unreadable. */
async function serveRelease(): Promise<ServeRelease | null> {
  // The installed-release observation belongs to the native mounted serve only, reached through
  // its public short entry or its mounted path.
  if (!publicEntry() && !/^\/r\/[a-z0-9]+\/x\/colab(?:\/|$)/.test(location.pathname)) return null;
  const root = publicEntry() ? entryMount() : new URL(location.href);
  root.hash = '';
  root.search = '';
  const response = await fetch(new URL('api/serve-release', root), {
    cache: 'no-store',
    credentials: 'same-origin',
    signal: AbortSignal.timeout(5000),
  });
  return response.ok ? parseServeRelease(await response.json()) : null;
}

export interface BuildSources {
  loaded(): string | null | Promise<string | null>;
  served(): Promise<string | null>;
  /** Absent in tests that only cover the bundle comparison. */
  release?(): Promise<ServeRelease | null>;
  now(): number;
}

/**
 * Whether this tab runs an older app build than the server now serves. Stale only on a
 * real hash change of the same bundle: an offline read or a different page says nothing.
 * Callers ask on failure paths and navigation; there is no polling.
 */
export function createBuildWatch(
  sources: BuildSources = {
    loaded: runningEntry,
    served: servedEntry,
    release: serveRelease,
    now: () => performance.now(),
  },
) {
  let stale = false;
  let older: ServeRelease | null = null;
  let checking = false;
  let last = -Infinity;
  const listeners = new Set<() => void>();
  return {
    stale: () => stale,
    /** The serve is older than the installed release; reloading cannot fix this. */
    serveOlder: () => older,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => void listeners.delete(listener);
    },
    async check(): Promise<boolean> {
      if (stale || older || checking || sources.now() - last < CHECK_INTERVAL_MS) return stale;
      checking = true;
      last = sources.now();
      try {
        const loaded = await sources.loaded();
        const served = loaded ? await sources.served().catch(() => null) : null;
        const release = (await sources.release?.().catch(() => null)) ?? null;
        stale = !!loaded && !!served && isNewerBuild(loaded, served);
        if (release?.installed && release.installed !== release.running) older = release;
        if (stale || older) listeners.forEach((listener) => listener());
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

export function useServeOlder(watch = buildWatch): ServeRelease | null {
  return useSyncExternalStore(watch.subscribe, watch.serveOlder);
}
