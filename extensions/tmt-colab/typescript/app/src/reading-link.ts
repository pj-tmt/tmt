import { validShellAnchor } from './shell.js';
import { validPagePrefix } from './short-links.js';
/** Cosmetic navigation for owner/paired pages only. It never carries reader admission. */
const readingId = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
export function validReadingId(value: string): boolean {
  return readingId.test(value);
}

function validPath(path: string): boolean {
  return (
    (path.startsWith('/pages/') && validReadingId(path.slice(7))) ||
    (path.startsWith('/short/') && validPagePrefix(path.slice(7)))
  );
}
export function readingSearch(search: Record<string, unknown>): { read?: string } {
  return typeof search.read === 'string' && validReadingId(search.read)
    ? { read: search.read }
    : {};
}

/** A page opens at one cosmetic target: a reading, or one exact element of a shell item. Never
 * both, and never anything else. */
export type PageHint = { read: string } | { anchor: string };
export function pageSearch(search: Record<string, unknown>): { read?: string; anchor?: string } {
  const read = readingSearch(search).read;
  const anchor =
    typeof search.anchor === 'string' && validShellAnchor(search.anchor)
      ? search.anchor
      : undefined;
  if (read !== undefined && anchor !== undefined) return {};
  return read !== undefined ? { read } : anchor !== undefined ? { anchor } : {};
}

/** A router path with its cosmetic hint, when it carries exactly one valid one. */
export function splitPageHint(path: string): { route: string; hint?: PageHint } {
  const [route, query] = path.split('?');
  const search = new URLSearchParams(query);
  const hint: PageHint | undefined =
    search.size === 1 && validPath(route)
      ? search.has('read')
        ? readingSearch({ read: search.get('read') }).read !== undefined
          ? { read: search.get('read')! }
          : undefined
        : route.startsWith('/pages/') && pageSearch({ anchor: search.get('anchor') }).anchor
          ? { anchor: search.get('anchor')! }
          : undefined
      : undefined;
  return hint ? { route, hint } : { route: path };
}

/** Router-internal paths carry the cosmetic hint as search; owner URLs keep it in the fragment. */
export function ownerReadingHref(path: string, pathname: string, space: string): string {
  const { route: target, hint } = splitPageHint(path);
  const tail = hint ? ('read' in hint ? `&read=${hint.read}` : `&anchor=${hint.anchor}`) : '';
  return `${pathname}#space=${space}${target === '/' ? '' : `&path=${encodeURIComponent(target)}`}${tail}`;
}

/** Refuse extra/duplicate parameters, so a navigation hint cannot hide a stray capability. */
export function ownerPageHint(url: URL, space: string, page?: string): PageHint | undefined {
  if (url.search || url.pathname.endsWith('/read')) return;
  const fields = new URLSearchParams(url.hash.slice(1));
  const keys = [...fields.keys()].sort().join(',');
  if (
    (keys !== 'path,read,space' && keys !== 'anchor,path,space') ||
    fields.get('space') !== space ||
    !validPath(fields.get('path') ?? '') ||
    (page !== undefined && fields.get('path') !== `/pages/${page}`)
  )
    return;
  if (keys === 'path,read,space') {
    const read = fields.get('read')!;
    return validReadingId(read) ? { read } : undefined;
  }
  // An anchor names an element inside a full page, so it never rides a short link.
  const anchor = fields.get('anchor')!;
  return fields.get('path')!.startsWith('/pages/') && validShellAnchor(anchor)
    ? { anchor }
    : undefined;
}

/** The reading hint alone; the anchor hint belongs to the page's own reveal. */
export function ownerReadingHint(url: URL, space: string, page?: string): string | undefined {
  const hint = ownerPageHint(url, space, page);
  return hint && 'read' in hint ? hint.read : undefined;
}
