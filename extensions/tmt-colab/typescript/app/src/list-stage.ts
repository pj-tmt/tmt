import type { PageSummary } from './transport.js';

/** The stages the page list tabs name; the owner derives them from a page's tasks. */
export type PageStage = 'in-progress' | 'discussion' | 'done';
/** What the page list's tabs show: a stage, or every active page. */
export type HomeStage = PageStage | 'all';
export const HOME_STAGES: readonly HomeStage[] = ['all', 'discussion', 'in-progress', 'done'];
/** The tab a bare list URL opens on (every active page); the route search keeps any other. */
export const DEFAULT_VIEW: HomeStage = 'all';
/** A tab of the owner's list: a stage, the pages that need you, or the archived ones. */
export type HomeView = HomeStage | 'needs' | 'archived';

/**
 * The stage a page's summary says. A fresh summary without work is a page with no task, so it is in
 * discussion; a page without a summary has not reported yet and is at no stage.
 */
export function pageStage(page: PageSummary): PageStage | undefined {
  const summary = page.summary;
  return summary ? (summary.work?.stage ?? 'discussion') : undefined;
}

/** How many of `pages` are at each stage; a page that has not reported counts under none. */
export function stageCounts(pages: readonly PageSummary[]): Record<PageStage, number> {
  const counts = { 'in-progress': 0, discussion: 0, done: 0 };
  for (const page of pages) {
    const stage = pageStage(page);
    if (stage) counts[stage] += 1;
  }
  return counts;
}

/**
 * The last `stage:<name>` word of the free text, taken out of it: a stage is one state with the
 * tabs, so it never stays in the search as a token. Any other word, `stage:` with an unknown name
 * included, stays as text.
 */
export function takeStage(q: string): { q: string; stage?: HomeStage } {
  let stage: HomeStage | undefined;
  const rest: string[] = [];
  for (const word of q.split(/\s+/).filter(Boolean)) {
    const named = /^stage:(.+)$/i.exec(word)?.[1]?.toLowerCase();
    const known = HOME_STAGES.find((name) => name === named);
    if (known) stage = known;
    else rest.push(word);
  }
  return { q: rest.join(' '), ...(stage ? { stage } : {}) };
}
