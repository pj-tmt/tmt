import { projectNameError } from './projects.js';
import type { PageSummary } from './transport.js';

/** The project the list is scoped to on this device, per space. Convenience only: a refused
 * storage access means the list opens on All pages. */
const prefix = 'tmt-colab-project-scope-v1:';

export function readScope(space: string | undefined): string | undefined {
  try {
    const stored = localStorage.getItem(prefix + (space ?? ''));
    return stored !== null && projectNameError(stored) === undefined ? stored : undefined;
  } catch {
    return undefined;
  }
}
/** `undefined` is All pages: the preference is removed, so Colab opens unscoped. */
export function writeScope(space: string | undefined, scope: string | undefined): void {
  try {
    if (scope === undefined) localStorage.removeItem(prefix + (space ?? ''));
    else localStorage.setItem(prefix + (space ?? ''), scope);
  } catch {
    /* The route search still scopes this visit. */
  }
}

/** A scope names a project the space has; a link to one that is gone shows All pages. */
export function knownScope(
  scope: string | undefined,
  projects: readonly string[] | undefined,
): string | undefined {
  return scope !== undefined && projects?.includes(scope) ? scope : undefined;
}

/** The pages in the scope; no scope keeps every page. */
export function inScope(
  pages: readonly PageSummary[],
  scope: string | undefined,
): readonly PageSummary[] {
  return scope === undefined ? pages : pages.filter((page) => page.projects?.includes(scope));
}

export interface ScopeRow {
  /** `undefined` is All pages. */
  name: string | undefined;
  count: number;
  /** Pages in it with an item waiting on the viewer. */
  waiting: number;
}
/** All pages, then each project of the space (those with no page too) in code point order, each with
 * the pages shown in it and how many of them wait on the viewer. */
export function scopeRows(pages: readonly PageSummary[], projects: readonly string[]): ScopeRow[] {
  const waiting = (list: readonly PageSummary[]) =>
    list.filter((page) => page.summary?.waiting).length;
  return [
    { name: undefined, count: pages.length, waiting: waiting(pages) },
    ...projects.map((name) => {
      const members = pages.filter((page) => page.projects?.includes(name));
      return { name, count: members.length, waiting: waiting(members) };
    }),
  ];
}
