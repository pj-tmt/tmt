import { generatedId, requireValue } from '@tmt/colab-client';
import { isHostedOrigin } from './hosted-location.js';
import { validReadingId, type PageHint } from './reading-link.js';
import { validShellAnchor } from './shell.js';

/** Remote supplies the internal mount; public location carries only a display target. The hosted
 * static origin serves the same paths with no Remote mount, so it is never a public entry. */
export function publicEntry(path = location.pathname, origin = location.origin): boolean {
  return (
    !isHostedOrigin(origin) &&
    (path === '/colab' || path === '/colab/' || /^\/(?:p|read)\/[A-Za-z0-9_-]{4,64}$/.test(path))
  );
}
export function entryMount(): URL {
  const supplied = document.querySelector<HTMLMetaElement>('meta[name="tmt-colab-mount"]')?.content;
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
/** One thread, one reading or one shell element, or nothing: anything else refuses. */
export function entryTarget(hash = location.hash): EntryTarget | null {
  requireValue(hash === '' || hash.startsWith('#'));
  const fields = new URLSearchParams(hash.slice(1));
  const keys = [...fields.keys()];
  requireValue(keys.length <= 1 && keys.every((key) => ['t', 'read', 'anchor'].includes(key)));
  if (!keys.length) return null;
  const key = keys[0];
  requireValue(fields.getAll(key).length === 1);
  const value = fields.get(key)!;
  if (key === 't') {
    generatedId(value);
    return { thread: value };
  }
  if (key === 'read') {
    requireValue(validReadingId(value));
    return { read: value };
  }
  requireValue(validShellAnchor(value));
  return { anchor: value };
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
  return 'read' in target ? `#read=${target.read}` : `#anchor=${target.anchor}`;
}
