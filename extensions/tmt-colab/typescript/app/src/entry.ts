import { generatedId, requireValue } from '@tmt/colab-client';
import { isHostedOrigin } from './hosted-location.js';
import { hintFragment, pageSearch, type PageHint } from './reading-link.js';
import { validShellAnchor } from './shell.js';

/** Colab inside the one TMT web app at `/colab/...` (Remote's shell host): the paired mount its
 * reads go to, or none for the local preview. Set once by the module entry before it renders. */
let shellMount: URL | null | undefined;
export function hostInShell(mount: URL | null): void {
  shellMount = mount;
}
/** Whether Colab runs as a route of the one TMT web app (path routes under `/colab/`). */
export function inShell(): boolean {
  return shellMount !== undefined;
}

/** Remote supplies the internal mount; public location carries only a display target. The hosted
 * static origin serves the same paths with no Remote mount, so it is never a public entry. Inside
 * the TMT app a paired Colab reads through its mount exactly as a public entry does. */
export function publicEntry(path = location.pathname, origin = location.origin): boolean {
  if (shellMount !== undefined) return shellMount !== null;
  return (
    !isHostedOrigin(origin) &&
    (path === '/colab' || path === '/colab/' || /^\/(?:p|read)\/[A-Za-z0-9_-]{4,64}$/.test(path))
  );
}
export function entryMount(): URL {
  const supplied =
    shellMount?.pathname ??
    document.querySelector<HTMLMetaElement>('meta[name="tmt-colab-mount"]')?.content;
  const url = new URL(supplied ?? location.pathname, location.origin);
  if (
    url.origin !== location.origin ||
    !/^\/r\/[a-z2-7]{16}\/x\/colab\/$/.test(url.pathname) ||
    url.search ||
    url.hash
  )
    throw new Error('Invalid Colab mount');
  return url;
}

/** A public owner fragment names at most one display target; it never carries space or reader authority. */
export type EntryTarget = { thread: string } | PageHint;

/** Whether a fragment is exactly one display target of a short public link, or none. */
export function entryFragment(hash: string): boolean {
  try {
    entryTarget(hash);
    return true;
  } catch {
    return false;
  }
}
/** One thread, one reading, one shell element or one task, or nothing: anything else refuses. */
export function entryTarget(hash = location.hash): EntryTarget | null {
  requireValue(hash === '' || hash.startsWith('#'));
  const fields = new URLSearchParams(hash.slice(1));
  const keys = [...fields.keys()];
  requireValue(
    keys.length <= 1 && keys.every((key) => ['t', 'read', 'anchor', 'task'].includes(key)),
  );
  if (!keys.length) return null;
  const key = keys[0];
  requireValue(fields.getAll(key).length === 1);
  const value = fields.get(key)!;
  if (key === 't') {
    generatedId(value);
    return { thread: value };
  }
  // The page hints share one parse with the router's search, so each key has one shape.
  const hint = pageSearch({ [key]: value });
  requireValue(key in hint);
  return { [key]: value } as PageHint;
}

/** The keys this entry never takes from a link: the owner's own location and a reader's capability. */
const REFUSED_KEYS = ['space', 'path', 'seed'];

/** What a public link asks the page to show when it opens. A plain `#<id>`, the web's own form,
 * names the shell element like `#anchor=<id>`. Any other fragment that is not exactly one display
 * target is ignored and the page opens at the top; only one that carries a refused key refuses. */
export function entryOpenTarget(hash = location.hash): EntryTarget | null {
  try {
    return entryTarget(hash);
  } catch (error) {
    if (hash.startsWith('#') && validShellAnchor(hash.slice(1))) return { anchor: hash.slice(1) };
    const keys = [...new URLSearchParams(hash.slice(1)).keys()];
    if (keys.some((key) => REFUSED_KEYS.includes(key))) throw error;
    return null;
  }
}

/** The canonical fragment of a display target, the only form kept in the location. */
export function entryHash(target: EntryTarget | null): string {
  if (!target) return '';
  if ('thread' in target) return `#t=${target.thread}`;
  return hintFragment(target);
}
