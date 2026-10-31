import type { PageState } from './bootstrap.js';
import type { PageSummary } from './transport.js';
import { taskNumber, type Work } from './work.js';

/** A sub-page row of the work panel. Its link is still stored as a task of its parent that
 * follows it (#4118); `carrier` names that task so Detach can remove it. */
export interface SubpageRow {
  id: string;
  /** The page whose task holds the link. */
  parent: string;
  carrier: string;
  title?: string;
  state?: PageState | null;
}
/** Direct sub-page progress: each sub-page counts once, by its own manual state, never recursively. */
export interface SubpageCounts {
  done: number;
  inProgress: number;
  todo: number;
  unknown: number;
  total: number;
}
/** Why a page cannot be attached under this one; the owner's admission decides again. */
export type AttachRefusal =
  | { kind: 'self' }
  | { kind: 'attached'; parentTitle?: string }
  | { kind: 'loop' };

/**
 * The sub-page tree as the owner list shows it. This page's own work names its direct sub-pages in
 * task order; a deeper level comes from the list's parent links (`summary.parent`), ordered by the
 * parent's task number, which is the parent's creation order. A page the list does not name has no
 * title or state and no children here: nothing is read to find them.
 */
export function subpageTree(
  pageId: string,
  work: Work | undefined,
  pages: readonly PageSummary[] | undefined,
): {
  top: readonly SubpageRow[];
  childrenOf(id: string): readonly SubpageRow[];
  counts(rows: readonly SubpageRow[]): SubpageCounts;
  refusal(candidate: string): AttachRefusal | undefined;
} {
  const byId = new Map((pages ?? []).map((page) => [page.id, page]));
  const row = (id: string, parent: string, carrier: string): SubpageRow => {
    const page = byId.get(id);
    return { id, parent, carrier, title: page?.title, state: page?.state };
  };
  const seen = new Set<string>();
  const top: SubpageRow[] = [];
  for (const task of work?.order ?? []) {
    const child = work?.tasks[task]?.child;
    // One row per page even when two tasks follow it; the first task in order carries it.
    if (child === undefined || seen.has(child)) continue;
    seen.add(child);
    top.push(row(child, pageId, task));
  }
  const below = new Map<string, SubpageRow[]>();
  for (const page of pages ?? []) {
    const link = page.summary?.parent;
    if (!link || link.page === pageId) continue;
    const rows = below.get(link.page) ?? [];
    rows.push(row(page.id, link.page, link.task));
    below.set(link.page, rows);
  }
  for (const rows of below.values())
    rows.sort(
      (a, b) =>
        (taskNumber(a.carrier) ?? 0) - (taskNumber(b.carrier) ?? 0) || (a.id < b.id ? -1 : 1),
    );
  const childrenOf = (id: string) => (id === pageId ? top : (below.get(id) ?? []));
  const counts = (rows: readonly SubpageRow[]): SubpageCounts => {
    const found = { done: 0, inProgress: 0, todo: 0, unknown: 0, total: rows.length };
    for (const { state } of rows) {
      if (state === 'done') found.done += 1;
      else if (state === 'in-progress') found.inProgress += 1;
      else if (state === 'todo') found.todo += 1;
      else found.unknown += 1;
    }
    return found;
  };
  const ancestors = new Set<string>();
  // The list's parent links can loop after a writers' race; the walk stops at a repeat.
  for (
    let link = byId.get(pageId)?.summary?.parent;
    link && !ancestors.has(link.page);
    link = byId.get(link.page)?.summary?.parent
  )
    ancestors.add(link.page);
  const refusal = (candidate: string): AttachRefusal | undefined => {
    if (candidate === pageId) return { kind: 'self' };
    if (ancestors.has(candidate)) return { kind: 'loop' };
    const link = byId.get(candidate)?.summary?.parent;
    if (link) return { kind: 'attached', parentTitle: byId.get(link.page)?.title };
    if (seen.has(candidate)) return { kind: 'attached', parentTitle: byId.get(pageId)?.title };
    return undefined;
  };
  return { top, childrenOf, counts, refusal };
}
