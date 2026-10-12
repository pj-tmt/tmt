import { LABEL_POINTS, LABELS_PER_PAGE } from './bootstrap.js';
import { orderPages, pageTitle } from './page-index.js';
import type { PageSummary } from './transport.js';

/** Pages one `page.labels` request may name; `limits.rs` and the contract's Page labels hold it too. */
export const LABEL_REQUEST_PAGES = 64;

export type PageSort = 'updated' | 'title';
/** What the page list shows; the route search is its only stored form. */
export interface PageFilter {
  /** Label names a page must all carry. */
  labels: readonly string[];
  /** Words that must each appear in the title or a label, ignoring case. */
  text: string;
}
export interface LabelChange {
  pageId: string;
  /** The page's whole new set. */
  labels: readonly string[];
}
export type LabelNameError = 'empty' | 'long' | 'form';

/**
 * The owner store's grammar: single interior spaces, no control character and no other Unicode
 * `White_Space` (not `\s`, which also takes U+FEFF); `label-name-v1.json` pins both sides.
 */
export function labelNameError(name: string): LabelNameError | undefined {
  const points = [...name].length;
  if (points === 0) return 'empty';
  if (points > LABEL_POINTS) return 'long';
  if (
    name.startsWith(' ') ||
    name.endsWith(' ') ||
    name.includes('  ') ||
    /\p{Cc}|(?! )\p{White_Space}/u.test(name)
  )
    return 'form';
  return undefined;
}

/** Code point order, as the owner store sorts a page's set. */
export function compareLabels(a: string, b: string): number {
  const x = [...a],
    y = [...b];
  for (let index = 0; index < Math.min(x.length, y.length); index++) {
    const difference = x[index].codePointAt(0)! - y[index].codePointAt(0)!;
    if (difference) return difference;
  }
  return x.length - y.length;
}

export function labelCounts(pages: readonly PageSummary[]): { name: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const page of pages)
    for (const name of page.labels ?? []) counts.set(name, (counts.get(name) ?? 0) + 1);
  return [...counts]
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => compareLabels(a.name, b.name));
}

export function filterPages(
  pages: readonly PageSummary[],
  filter: PageFilter,
  sort: PageSort,
): PageSummary[] {
  const words = filter.text.toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const kept = pages.filter((page) => {
    const labels = page.labels ?? [];
    if (!filter.labels.every((name) => labels.includes(name))) return false;
    const haystacks = [pageTitle(page), ...labels].map((value) => value.toLocaleLowerCase());
    return words.every((word) => haystacks.some((value) => value.includes(word)));
  });
  return sort === 'title'
    ? [...kept].sort(
        (a, b) => pageTitle(a).localeCompare(pageTitle(b), 'en') || a.id.localeCompare(b.id, 'en'),
      )
    : orderPages(kept);
}

/** One page's whole set with a name added or removed; a full page is refused, never truncated. */
export function withLabel(
  page: PageSummary,
  name: string,
  present: boolean,
): LabelChange | 'capacity' {
  const rest = (page.labels ?? []).filter((label) => label !== name);
  const labels = present ? [...rest, name] : rest;
  if (labels.length > LABELS_PER_PAGE) return 'capacity';
  return { pageId: page.id, labels: labels.sort(compareLabels) };
}

/**
 * Every page that carries `from`, archived included, whatever the list shows. A rename onto a name
 * the page already has keeps one label; `to` undefined deletes. Pure on the pages it is given, so
 * a retry that re-reads them plans only what is left.
 */
export function relabel(pages: readonly PageSummary[], from: string, to?: string): LabelChange[] {
  if (from === to) return [];
  return pages
    .filter((page) => page.labels?.includes(from))
    .map((page) => {
      const rest = page.labels!.filter((label) => label !== from && label !== to);
      return {
        pageId: page.id,
        labels: (to === undefined ? rest : [...rest, to]).sort(compareLabels),
      };
    });
}

/** Ascending page IDs in requests of at most `LABEL_REQUEST_PAGES`; the first page initiates. */
export function labelRequests(changes: readonly LabelChange[]): LabelChange[][] {
  const sorted = [...changes].sort((a, b) =>
    a.pageId < b.pageId ? -1 : a.pageId > b.pageId ? 1 : 0,
  );
  const requests: LabelChange[][] = [];
  for (let at = 0; at < sorted.length; at += LABEL_REQUEST_PAGES)
    requests.push(sorted.slice(at, at + LABEL_REQUEST_PAGES));
  return requests;
}

export interface LabelWriter {
  setLabels(changes: readonly LabelChange[]): Promise<void>;
}
export interface LabelOutcome {
  /** Pages in the requests that completed. */
  updated: number;
  total: number;
  failure?: unknown;
}
/**
 * Sequential requests, each one store transaction on the owner: atomic per request, not across
 * them. A failure stops the batch and reports how far it got.
 */
export async function applyLabelChanges(
  writer: LabelWriter,
  changes: readonly LabelChange[],
): Promise<LabelOutcome> {
  const outcome: LabelOutcome = { updated: 0, total: changes.length };
  for (const request of labelRequests(changes)) {
    try {
      await writer.setLabels(request);
    } catch (failure) {
      return { ...outcome, failure };
    }
    outcome.updated += request.length;
  }
  return outcome;
}
