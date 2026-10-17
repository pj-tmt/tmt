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

/** Router-internal paths carry the cosmetic hint as search; owner URLs keep it in the fragment. */
export function ownerReadingHref(path: string, pathname: string, space: string): string {
  const [route, query] = path.split('?');
  const search = new URLSearchParams(query);
  const read =
    search.size === 1 && validPath(route)
      ? readingSearch({ read: search.get('read') }).read
      : undefined;
  const target = read ? route : path;
  return `${pathname}#space=${space}${target === '/' ? '' : `&path=${encodeURIComponent(target)}`}${read ? `&read=${read}` : ''}`;
}

/** Refuse extra/duplicate parameters, so a navigation hint cannot hide a stray capability. */
export function ownerReadingHint(url: URL, space: string, page?: string): string | undefined {
  if (url.search || url.pathname.endsWith('/read')) return;
  const fields = new URLSearchParams(url.hash.slice(1));
  if (
    [...fields.keys()].sort().join(',') !== 'path,read,space' ||
    fields.get('space') !== space ||
    !validPath(fields.get('path') ?? '') ||
    (page !== undefined && fields.get('path') !== `/pages/${page}`)
  )
    return;
  const read = fields.get('read')!;
  return validReadingId(read) ? read : undefined;
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
