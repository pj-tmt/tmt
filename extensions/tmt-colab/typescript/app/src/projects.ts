import { PROJECT_POINTS, PROJECTS_PER_PAGE } from './bootstrap.js';
import { orderPages, pageTitle } from './page-index.js';
import type { PageSummary } from './transport.js';

/** Pages one `page.projects` request may name; `limits.rs` and the contract's Projects section hold it too. */
export const PROJECT_REQUEST_PAGES = 64;

export type PageSort = 'updated' | 'title';
/** What the page list shows; the route search is its only stored form. */
export interface PageFilter {
  /** Project names a page must all be in. */
  projects: readonly string[];
  /** Creators a page may have: any one of them (a page has one), none given means any. */
  authors: readonly string[];
  /** Words that must each appear in the title, a project or the creator, ignoring case. */
  text: string;
  /** Only the pages with an item waiting on the viewer (the owner's `waiting`, `needsYou > 0`). */
  needsYou?: boolean;
}
export interface ProjectChange {
  pageId: string;
  /** The page's whole new set. */
  projects: readonly string[];
}
/** One change to the space's projects, independent of any page (`project.manage`). */
export type ProjectManage =
  | { action: 'create' | 'delete'; name: string }
  | { action: 'rename'; name: string; to: string };
export type ProjectNameError = 'empty' | 'long' | 'form';

/**
 * The owner store's grammar: single interior spaces, no control character and no other Unicode
 * `White_Space` (not `\s`, which also takes U+FEFF); `project-name-v1.json` pins both sides.
 */
export function projectNameError(name: string): ProjectNameError | undefined {
  const points = [...name].length;
  if (points === 0) return 'empty';
  if (points > PROJECT_POINTS) return 'long';
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
export function compareProjects(a: string, b: string): number {
  const x = [...a],
    y = [...b];
  for (let index = 0; index < Math.min(x.length, y.length); index++) {
    const difference = x[index].codePointAt(0)! - y[index].codePointAt(0)!;
    if (difference) return difference;
  }
  return x.length - y.length;
}

/** Creators with their page counts, in code point order; pages without a known creator add none. */
export function authorCounts(pages: readonly PageSummary[]): { name: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const page of pages) {
    const author = page.summary?.author;
    if (author) counts.set(author, (counts.get(author) ?? 0) + 1);
  }
  return [...counts]
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => compareProjects(a.name, b.name));
}

/**
 * Projects with the number of `pages` that are in them, in code point order. `named` adds the
 * space's projects that no given page is in, so an empty project is still listed.
 */
export function projectCounts(
  pages: readonly PageSummary[],
  named: readonly string[] = [],
): { name: string; count: number }[] {
  const counts = new Map<string, number>(named.map((name) => [name, 0]));
  for (const page of pages)
    for (const name of page.projects ?? []) counts.set(name, (counts.get(name) ?? 0) + 1);
  return [...counts]
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => compareProjects(a.name, b.name));
}

/** The pages with an item waiting on the viewer; an unknown summary is never counted. */
export function waitingCount(pages: readonly PageSummary[]): number {
  return pages.filter((page) => page.summary?.waiting).length;
}

export function filterPages(
  pages: readonly PageSummary[],
  filter: PageFilter,
  sort: PageSort,
): PageSummary[] {
  const words = filter.text.toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const kept = pages.filter((page) => {
    if (filter.needsYou && !page.summary?.waiting) return false;
    const projects = page.projects ?? [];
    if (!filter.projects.every((name) => projects.includes(name))) return false;
    const author = page.summary?.author;
    if (filter.authors.length && (!author || !filter.authors.includes(author))) return false;
    const haystacks = [pageTitle(page), ...projects, ...(author ? [author] : [])].map((value) =>
      value.toLocaleLowerCase(),
    );
    return words.every((word) => haystacks.some((value) => value.includes(word)));
  });
  return sort === 'title'
    ? [...kept].sort(
        (a, b) => pageTitle(a).localeCompare(pageTitle(b), 'en') || a.id.localeCompare(b.id, 'en'),
      )
    : orderPages(kept);
}

/** One page's whole set with a name added or removed; a full page is refused, never truncated. */
export function withProject(
  page: PageSummary,
  name: string,
  present: boolean,
): ProjectChange | 'capacity' {
  const rest = (page.projects ?? []).filter((project) => project !== name);
  const projects = present ? [...rest, name] : rest;
  if (projects.length > PROJECTS_PER_PAGE) return 'capacity';
  return { pageId: page.id, projects: projects.sort(compareProjects) };
}

/** Ascending page IDs in requests of at most `PROJECT_REQUEST_PAGES`; the first page initiates. */
export function projectRequests(changes: readonly ProjectChange[]): ProjectChange[][] {
  const sorted = [...changes].sort((a, b) =>
    a.pageId < b.pageId ? -1 : a.pageId > b.pageId ? 1 : 0,
  );
  const requests: ProjectChange[][] = [];
  for (let at = 0; at < sorted.length; at += PROJECT_REQUEST_PAGES)
    requests.push(sorted.slice(at, at + PROJECT_REQUEST_PAGES));
  return requests;
}

export interface ProjectWriter {
  setProjects(changes: readonly ProjectChange[]): Promise<void>;
  manageProject(change: ProjectManage): Promise<void>;
}
export interface ProjectOutcome {
  /** Pages in the requests that completed. */
  updated: number;
  total: number;
  failure?: unknown;
}
/**
 * Sequential requests, each one store transaction on the owner: atomic per request, not across
 * them. A failure stops the batch and reports how far it got.
 */
export async function applyProjectChanges(
  writer: ProjectWriter,
  changes: readonly ProjectChange[],
): Promise<ProjectOutcome> {
  const outcome: ProjectOutcome = { updated: 0, total: changes.length };
  for (const request of projectRequests(changes)) {
    try {
      await writer.setProjects(request);
    } catch (failure) {
      return { ...outcome, failure };
    }
    outcome.updated += request.length;
  }
  return outcome;
}
