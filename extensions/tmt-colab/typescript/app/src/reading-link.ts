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

/** A page opens at one cosmetic target: a reading or one exact element of a shell item. Never more
 * than one, and never anything else. It only picks what the page shows; it admits nothing. */
export type PageHint = { read: string } | { anchor: string };
const HINT_KEYS = ['read', 'anchor'] as const;
type HintKey = (typeof HINT_KEYS)[number];
/** The valid value of one hint key, or undefined: each key keeps its own shape. */
function hintValue(key: HintKey, value: unknown): string | undefined {
  if (typeof value !== 'string') return;
  const valid = key === 'read' ? validReadingId(value) : validShellAnchor(value);
  return valid ? value : undefined;
}
function hintOf(key: HintKey, value: string): PageHint {
  return key === 'read' ? { read: value } : { anchor: value };
}
/** The one key and value a hint carries, for every location form. */
function hintPair(hint: PageHint): [HintKey, string] {
  return 'read' in hint ? ['read', hint.read] : ['anchor', hint.anchor];
}
/** A hint as router search (`?anchor=decisions`) or as a public fragment (`#anchor=decisions`). */
export const hintSearch = (hint: PageHint): string => '?' + hintPair(hint).join('=');
export const hintFragment = (hint: PageHint): string => '#' + hintPair(hint).join('=');
export function pageSearch(search: Record<string, unknown>): {
  read?: string;
  anchor?: string;
} {
  const found = HINT_KEYS.flatMap((key) => {
    const value = hintValue(key, search[key]);
    return value === undefined ? [] : [[key, value] as const];
  });
  // Two valid hints together are no hint at all.
  return found.length === 1 ? { [found[0][0]]: found[0][1] } : {};
}

/** The shell chapter whose panel is open (`?open=<pageId>`): a page ID and nothing else, so a stray
 * value is dropped. It only says which chapter of the shell shows its panel; it never admits anything. */
export function openSearch(search: Record<string, unknown>): { open?: string } {
  return typeof search.open === 'string' && validReadingId(search.open)
    ? { open: search.open }
    : {};
}

/** A router path with its cosmetic hint, when it carries exactly one valid one. */
export function splitPageHint(path: string): { route: string; hint?: PageHint } {
  const [route, query] = path.split('?');
  const search = new URLSearchParams(query);
  const key = HINT_KEYS.find((name) => search.has(name));
  const value = key === undefined ? undefined : hintValue(key, search.get(key));
  const hint: PageHint | undefined =
    search.size === 1 && validPath(route) && key !== undefined && value !== undefined
      ? hintOf(key, value)
      : undefined;
  return hint ? { route, hint } : { route: path };
}

/** Router-internal paths carry the cosmetic hint as search; owner URLs keep it in the fragment. */
export function ownerReadingHref(path: string, pathname: string, space: string): string {
  const { route: target, hint } = splitPageHint(path);
  const tail = hint ? `&${hintPair(hint).join('=')}` : '';
  return `${pathname}#space=${space}${target === '/' ? '' : `&path=${encodeURIComponent(target)}`}${tail}`;
}

/** Refuse extra/duplicate parameters, so a navigation hint cannot hide a stray capability. */
export function ownerPageHint(url: URL, space: string, page?: string): PageHint | undefined {
  if (url.search || url.pathname.endsWith('/read')) return;
  const fields = new URLSearchParams(url.hash.slice(1));
  const keys = [...fields.keys()].sort().join(',');
  const key = HINT_KEYS.find((name) => keys === [name, 'path', 'space'].sort().join(','));
  if (
    key === undefined ||
    fields.get('space') !== space ||
    !validPath(fields.get('path') ?? '') ||
    (page !== undefined && fields.get('path') !== `/pages/${page}`)
  )
    return;
  // A short link keeps its cosmetic hint through resolution to the admitted full page.
  const value = hintValue(key, fields.get(key));
  return value === undefined ? undefined : hintOf(key, value);
}

/** The reading hint alone; the anchor hint belongs to the page's own reveal. */
export function ownerReadingHint(url: URL, space: string, page?: string): string | undefined {
  const hint = ownerPageHint(url, space, page);
  return hint && 'read' in hint ? hint.read : undefined;
}
