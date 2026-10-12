import type { PageFilter } from './labels.js';
import { pageTitle } from './page-index.js';
import type { PageSummary } from './transport.js';

/** The most results one query returns. */
export const SWITCHER_RESULTS = 50;

export interface SwitcherResult {
  /** The list row as it is: title, labels, sharing, the owner's summary. */
  page: PageSummary;
  /** The field the free text matched (that of its weakest word); absent without free text. */
  matched?: 'title' | 'label' | 'author';
}
export interface RankOptions {
  /** A page that is never offered, such as the one the switcher was opened on (applied before `limit`). */
  exclude?: string;
  limit?: number;
}

/** How well one word matches a page, best (0) to weakest: title prefix, a title word's prefix,
 * title contains, a label, the creator. */
const STRENGTHS = ['title', 'title', 'title', 'label', 'author'] as const;
const fold = (value: string) => value.toLocaleLowerCase();
function wordStrength(word: string, title: string, labels: readonly string[], author?: string) {
  const name = fold(title);
  if (name.startsWith(word)) return 0;
  if (name.split(/[^\p{L}\p{N}]+/u).some((part) => part.startsWith(word))) return 1;
  if (name.includes(word)) return 2;
  if (labels.some((label) => fold(label).includes(word))) return 3;
  if (author && fold(author).includes(word)) return 4;
  return undefined;
}

/** `max(opened, updated)`; a page with neither is never recent. */
const recency = (page: PageSummary, opened: ReadonlyMap<string, number>) =>
  Math.max(opened.get(page.id) ?? -Infinity, page.lastUpdateAtMs ?? -Infinity);

/**
 * The pages a quick switcher offers for `query`, best first. The filter is the page list's own:
 * every `labels` name must be carried, any one of `authors` is the creator, and each free-text word
 * must appear in the title, a label or the creator (ignoring case). Archived pages are not offered.
 *
 * Without free text, pages with a proposal awaiting you (`summary.waiting`) come first, then the
 * most recently opened or updated. With free text, match strength comes first (its weakest word),
 * then waiting, then recency. Ties break by title, then ID.
 */
export function rankPages(
  pages: readonly PageSummary[],
  query: PageFilter,
  opened: ReadonlyMap<string, number>,
  { exclude, limit = SWITCHER_RESULTS }: RankOptions = {},
): SwitcherResult[] {
  const words = fold(query.text).split(/\s+/).filter(Boolean);
  const ranked: { result: SwitcherResult; strength: number; waiting: boolean; at: number }[] = [];
  for (const page of pages) {
    if (page.archived || page.id === exclude) continue;
    const labels = page.labels ?? [];
    const author = page.summary?.author;
    if (!query.labels.every((name) => labels.includes(name))) continue;
    if (query.authors.length && (!author || !query.authors.includes(author))) continue;
    const title = pageTitle(page);
    let strength = 0;
    for (const word of words) {
      const best = wordStrength(word, title, labels, author);
      if (best === undefined) {
        strength = -1;
        break;
      }
      strength = Math.max(strength, best);
    }
    if (strength < 0) continue;
    ranked.push({
      result: { page, ...(words.length ? { matched: STRENGTHS[strength] } : {}) },
      strength,
      waiting: page.summary?.waiting === true,
      at: recency(page, opened),
    });
  }
  ranked.sort(
    (a, b) =>
      a.strength - b.strength ||
      Number(b.waiting) - Number(a.waiting) ||
      (a.at === b.at ? 0 : a.at === -Infinity ? 1 : b.at === -Infinity ? -1 : b.at - a.at) ||
      pageTitle(a.result.page).localeCompare(pageTitle(b.result.page), 'en') ||
      a.result.page.id.localeCompare(b.result.page.id, 'en'),
  );
  return ranked.slice(0, Math.max(0, Math.floor(limit))).map((value) => value.result);
}
