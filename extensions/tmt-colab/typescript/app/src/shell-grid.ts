import type { ReadingDetails } from './reading-details.js';
import type { Shell, ShellItem } from './shell.js';
import type { PageSummary } from './transport.js';

/** One grid cell of a shell page: the item plus what the space list knows of its source page. */
export interface ShellCell {
  id: string;
  kind: ShellItem['kind'];
  pageId: string;
  anchor?: string;
  readingId?: string;
  /** The source page's title; empty while it is unavailable. */
  title: string;
  titleMissing: boolean;
  /** The source page is not in the space list (deleted or unreadable). The item stays listed. */
  unavailable: boolean;
  archived: boolean;
  /** The source page's last edit on the owner clock, when the space list knows it. */
  lastUpdateAtMs: number | null;
  /** Pending items on the source page, from its owner summary; 0 where none is known. */
  needsYou: number;
  note?: string;
  audience?: string;
}

/** Items in `order`, each matched to its source page by ID. A reference grants no access: a page
 * the space list does not show is unavailable, however the shell names it. */
export function shellCells(shell: Shell, pages: readonly PageSummary[]): ShellCell[] {
  const byId = new Map(pages.map((page) => [page.id, page]));
  return shell.order.map((id) => {
    const item = shell.items[id];
    const page = byId.get(item.pageId);
    return {
      id,
      kind: item.kind,
      pageId: item.pageId,
      ...(item.kind === 'anchor' ? { anchor: item.anchor } : {}),
      ...(item.kind === 'reading' ? { readingId: item.readingId } : {}),
      title: page?.title ?? '',
      titleMissing: page?.titleMissing === true,
      unavailable: page === undefined,
      archived: page?.archived === true,
      lastUpdateAtMs: page?.lastUpdateAtMs ?? null,
      needsYou: page?.summary?.needsYou ?? 0,
      ...(item.note === undefined ? {} : { note: item.note }),
      ...(item.audience === undefined ? {} : { audience: item.audience }),
    };
  });
}

/** What the owner chapter details say about a reading item: the source reading as the page lists
 * it, or that the page no longer lists it. Opening an item never changes this; only the page's own
 * `Mark read` does. */
export type ReadingView =
  | { kind: 'listed'; title: string; agent: string; why: string; state: 'unread' | 'read' }
  | { kind: 'missing' };

/** The source pages of the reading items the space list shows, each once, in order. A page the
 * list does not show is never asked about: a reference grants no access. */
export function readingPageIds(cells: readonly ShellCell[]): string[] {
  return [
    ...new Set(
      cells.filter((cell) => cell.kind === 'reading' && !cell.unavailable).map((c) => c.pageId),
    ),
  ];
}

/** A reading item's state from its source page's details. Unknown (`undefined`) unless the page's
 * details are ready: pending, unavailable, a failed read and a hosted reader show no state. */
export function readingView(
  cell: ShellCell,
  details: ReadingDetails | undefined,
): ReadingView | undefined {
  if (cell.kind !== 'reading' || cell.unavailable || details?.state !== 'ready') return undefined;
  const reading = details.readings.find((candidate) => candidate.id === cell.readingId);
  return reading
    ? {
        kind: 'listed',
        title: reading.title,
        agent: reading.agent,
        why: reading.why,
        state: reading.state,
      }
    : { kind: 'missing' };
}

/** Who wrote the summary and when. The writer's identity is never shown as a name; only the
 * Core-resolved display agent is, and without one just the time shows. */
export function shellByline(shell: Shell): { who?: string; timeMs: number } | undefined {
  const summary = shell.summary;
  if (!summary) return undefined;
  const { displayAgent, timeMs } = summary.attribution;
  return displayAgent ? { who: displayAgent.name, timeMs } : { timeMs };
}

/** How many distinct source pages were edited after the summary was written. A page named by
 * several items counts once; an unavailable page or one with no known edit time never counts.
 * Both times come from the one owner clock (the summary's `timeMs` is taken when the owner writes
 * it, `lastUpdateAtMs` when the owner accepts a page's content), so they compare directly. An edit
 * in the same millisecond as the summary is not later and does not count. */
export function pagesChangedSince(cells: readonly ShellCell[], summaryTimeMs: number): number {
  return new Set(
    cells
      .filter((cell) => !cell.unavailable && (cell.lastUpdateAtMs ?? -1) > summaryTimeMs)
      .map((cell) => cell.pageId),
  ).size;
}
