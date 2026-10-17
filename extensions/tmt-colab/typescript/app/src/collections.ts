export { validSectionId } from './section-link.js';
import { filterPages, type PageSort } from './labels.js';
import { orderPages, pageTitle } from './page-index.js';
import type { HomeSearch } from './home-search.js';
import type { PageSummary } from './transport.js';

export const COLLECTIONS = 64;
export const COLLECTION_ITEMS = 256;
export const SPACE_COLLECTION_ITEMS = 1000;
export const COLLECTION_NAME_POINTS = 64;
export const COLLECTION_NOTE_BYTES = 512;
export const COLLECTION_PERSON_BYTES = 128;
export type CollectionSort = 'manual' | PageSort;

/** Display-only owner data. These names grant neither page access nor notification authority. */
export interface CollectionItem {
  id: string;
  pageId: string;
  anchor?: string;
  note: string;
  audience: string;
}
export interface Collection {
  id: string;
  name: string;
  maintainer: string;
  archived: boolean;
  definition:
    | { kind: 'manual'; sort: CollectionSort }
    | { kind: 'smart'; search: HomeSearch; needsYou?: true };
}
export interface CollectionSeen {
  /** Manual item ID, or page ID in a smart collection. */
  id: string;
  atMs: number;
}
export interface CollectionDetail {
  collection: Collection;
  items: readonly CollectionItem[];
  seen: readonly CollectionSeen[];
}

export function collectionNameError(value: string): 'empty' | 'long' | 'form' | undefined {
  if (!value.trim()) return 'empty';
  if ([...value].length > COLLECTION_NAME_POINTS) return 'long';
  if (/\p{Cc}/u.test(value)) return 'form';
  return undefined;
}

/** The existing All-pages filter owns search semantics; summaries remain evidence, not defaults. */
export function smartPages(
  pages: readonly PageSummary[],
  search: HomeSearch,
  needsYou = false,
): { pages: PageSummary[]; pending: boolean } {
  const unique = [...new Map(pages.map((page) => [page.id, page])).values()];
  const candidates = unique.filter((page) => Boolean(page.archived) === Boolean(search.archived));
  const labelled = filterPages(
    candidates,
    { labels: search.label ?? [], authors: [], text: '' },
    'updated',
  );
  const pending = labelled.some(
    (page) =>
      ((needsYou || Boolean(search.by?.length) || Boolean(search.q)) &&
        page.summary === undefined) ||
      (Boolean(search.q) && page.titleMissing === true),
  );
  const matched = filterPages(
    candidates,
    {
      labels: search.label ?? [],
      authors: search.by ?? [],
      text: search.q ?? '',
    },
    search.sort ?? 'updated',
  );
  return {
    pages: needsYou
      ? matched.filter((page) => page.summary !== undefined && page.summary.needsYou > 0)
      : matched,
    pending,
  };
}

/** An unavailable reference stays in the collection. Repeated references keep their own identity. */
export function manualPages(
  items: readonly CollectionItem[],
  pages: readonly PageSummary[],
  sort: CollectionSort,
): { item: CollectionItem; page?: PageSummary }[] {
  const byId = new Map(pages.map((page) => [page.id, page]));
  const rows = items.map((item) => ({ item, page: byId.get(item.pageId) }));
  if (sort === 'manual') return rows;
  const ordered =
    sort === 'updated'
      ? orderPages(pages)
      : [...pages].sort(
          (a, b) =>
            pageTitle(a).localeCompare(pageTitle(b), 'en') || a.id.localeCompare(b.id, 'en'),
        );
  const position = new Map(ordered.map((page, index) => [page.id, index]));
  return rows.sort(
    (a, b) => (position.get(a.item.pageId) ?? Infinity) - (position.get(b.item.pageId) ?? Infinity),
  );
}
