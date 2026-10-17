import { ownerReadingHref, validReadingId } from './reading-link.js';
import { validPagePrefix } from './short-links.js';

/** Exact HTML fragment IDs, never selector syntax or semantic action admission. */
export function validSectionId(value: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/.test(value);
}
export function sectionSearch(search: Record<string, unknown>): { section?: string } {
  return typeof search.section === 'string' && validSectionId(search.section)
    ? { section: search.section }
    : {};
}
function pagePath(path: string): boolean {
  return (
    (path.startsWith('/pages/') && validReadingId(path.slice(7))) ||
    (path.startsWith('/short/') && validPagePrefix(path.slice(7)))
  );
}
/** Owner/paired cosmetic links only. Reader seeds and extra/duplicate fields are never accepted. */
export function ownerSectionHint(url: URL, space: string): string | undefined {
  if (url.search || url.pathname.endsWith('/read')) return;
  const fields = new URLSearchParams(url.hash.slice(1));
  if (
    [...fields.keys()].sort().join(',') !== 'path,section,space' ||
    fields.get('space') !== space ||
    !pagePath(fields.get('path') ?? '')
  )
    return;
  const section = fields.get('section')!;
  return validSectionId(section) ? section : undefined;
}
export function ownerSectionHref(path: string, pathname: string, space: string): string {
  const [route, query] = path.split('?');
  const search = new URLSearchParams(query);
  const section =
    search.size === 1 && pagePath(route)
      ? sectionSearch({ section: search.get('section') }).section
      : undefined;
  return section
    ? `${pathname}#space=${space}&path=${encodeURIComponent(route)}&section=${encodeURIComponent(section)}`
    : ownerReadingHref(path, pathname, space);
}
