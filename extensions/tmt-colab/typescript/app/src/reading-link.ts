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

/** Router-internal paths carry the cosmetic hint as search; owner URLs keep it in the fragment. */
export function ownerReadingHref(path: string, pathname: string, space: string): string {
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
  const target = hint ? route : path;
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

/** Build on the exact owner URL only, never on a hosted reader seed or an arbitrary fragment. */
export function readingPageUrl(
  url: string,
  space: string,
  page: string,
  read: string,
): string | undefined {
  const parsed = new URL(url);
  const fragment = `#space=${space}&path=${encodeURIComponent(`/pages/${page}`)}`;
  if (
    !validReadingId(read) ||
    parsed.search ||
    parsed.username ||
    parsed.password ||
    !['http:', 'https:'].includes(parsed.protocol) ||
    parsed.pathname.endsWith('/read') ||
    (parsed.hash !== fragment && !ownerReadingHint(parsed, space, page))
  )
    return;
  parsed.hash = `${fragment}&read=${read}`;
  return parsed.href;
}
