import type { HomeSummary } from './bootstrap.js';
import { orderPages } from './page-index.js';
import type { PageSummary } from './transport.js';

/** Recently updated pages that get a chapter of their own; older ones fold into one-line rows. */
export const RECENT_CHAPTERS = 10;

export interface TodayModel {
  /** Pages with items awaiting the owner, most recently updated first. */
  waiting: PageSummary[];
  /** The most recently updated other pages that keep a chapter. */
  recent: PageSummary[];
  /** Everything older than the visible chapters. */
  folded: PageSummary[];
  /** Counts over active (not archived) pages, each computed from the owner list summary. */
  counts: { waiting: number; updatedToday: number; pages: number; complete: boolean };
}

/** Today shows active pages only; the archived ones stay in All pages. */
export function todayModel(pages: readonly PageSummary[], now: number): TodayModel {
  const ordered = orderPages(pages.filter((page) => !page.archived));
  const waiting = ordered.filter((page) => page.summary?.waiting === true);
  const others = ordered.filter((page) => page.summary?.waiting !== true);
  const startOfDay = new Date(now);
  startOfDay.setHours(0, 0, 0, 0);
  return {
    waiting,
    recent: others.slice(0, RECENT_CHAPTERS),
    folded: others.slice(RECENT_CHAPTERS),
    counts: {
      complete: ordered.every((page) => page.summary !== undefined),
      waiting: waiting.reduce((total, page) => total + (page.summary?.needsYou ?? 0), 0),
      updatedToday: ordered.filter(
        (page) => page.lastUpdateAtMs != null && page.lastUpdateAtMs >= startOfDay.getTime(),
      ).length,
      pages: ordered.length,
    },
  };
}

/** Time of day from the viewer's own clock. */
export function greetingPart(now: number): 'morning' | 'afternoon' | 'evening' {
  const hour = new Date(now).getHours();
  return hour >= 5 && hour < 12 ? 'morning' : hour >= 12 && hour < 18 ? 'afternoon' : 'evening';
}

/** The viewer's date line, e.g. `Saturday, October 17`. */
export function dateLine(now: number): string {
  return new Intl.DateTimeFormat('en-US', {
    weekday: 'long',
    month: 'long',
    day: 'numeric',
  }).format(now);
}

/** Whether an active page changed after the summary was written, so the summary may be behind. */
export function pagesChangedSince(
  summary: Pick<HomeSummary, 'atMs'>,
  pages: readonly PageSummary[],
): boolean {
  return pages.some(
    (page) => !page.archived && page.lastUpdateAtMs != null && page.lastUpdateAtMs > summary.atMs,
  );
}
