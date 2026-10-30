import { canonical } from './closed.js';
import { listPages, validateHomeSearch, type HomeSearch } from './page-list-filter.js';
import type { PageSummary } from './transport.js';
import { text } from './strings.js';

export interface Shortcut {
  id: string;
  name: string;
  search: HomeSearch;
}
export interface ShortcutCollection {
  version: 1;
  migrated: boolean;
  items: Shortcut[];
}
export function shortcutNameError(name: string): 'empty' | 'form' | undefined {
  return !name.trim()
    ? 'empty'
    : [...name].length > 80 || /\p{Cc}/u.test(name)
      ? 'form'
      : undefined;
}
/** Scope becomes a normal project condition; the saved route never refers to another shortcut. */
export function shortcutSearch(search: HomeSearch): HomeSearch {
  const {
    scope,
    shortcut: _,
    ...validated
  } = validateHomeSearch(search as Record<string, unknown>);
  const project = [...new Set([...(scope ? [scope] : []), ...(validated.project ?? [])])];
  return Object.fromEntries(
    Object.entries({ ...validated, project: project.length ? project : undefined }).filter(
      ([, value]) => value !== undefined,
    ),
  );
}
export function readShortcuts(raw: string | null): ShortcutCollection {
  if (raw === null) return { version: 1, migrated: false, items: [] };
  const value: unknown = JSON.parse(raw);
  if (!value || typeof value !== 'object') throw new Error('Saved shortcuts cannot be read.');
  const record = value as ShortcutCollection;
  if (record.version !== 1 || typeof record.migrated !== 'boolean' || !Array.isArray(record.items))
    throw new Error('Saved shortcuts use an unsupported format.');
  const ids = new Set<string>();
  for (const item of record.items) {
    if (
      !item ||
      typeof item.id !== 'string' ||
      !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(item.id) ||
      ids.has(item.id) ||
      typeof item.name !== 'string' ||
      shortcutNameError(item.name) ||
      !item.search ||
      typeof item.search !== 'object' ||
      Array.isArray(item.search)
    )
      throw new Error('Saved shortcuts cannot be read.');
    if (canonical(shortcutSearch(item.search)) !== canonical(item.search))
      throw new Error('Saved shortcut filters cannot be read.');
    ids.add(item.id);
  }
  return record;
}

/** Counts are omitted when a pending field could change the matching set or attention. */
export function shortcutCounts(
  pages: readonly PageSummary[] | undefined,
  search: HomeSearch,
): { total?: number; attention?: number } {
  if (!pages) return {};
  const candidates = listPages(pages, {
    ...search,
    q: undefined,
    by: undefined,
    needs: undefined,
    stage: undefined,
    state: undefined,
  });
  const dependsOnSummary = !!(
    search.q ||
    search.by?.length ||
    search.needs ||
    (search.stage && search.stage !== 'all' && !search.archived)
  );
  if (
    candidates.some(
      (page) =>
        (dependsOnSummary && !page.summary) ||
        (!!search.q && page.titleMissing) ||
        (!!search.state && !!page.stateError),
    )
  )
    return {};
  const matches = listPages(pages, search);
  const total = matches.length;
  return matches.some((page) => !page.summary)
    ? { total }
    : { total, attention: matches.filter((page) => page.summary?.waiting).length };
}

/** The visible summary preserves every condition a saved list can carry. */
export function shortcutConditions(search: HomeSearch): string[] {
  const t = text.shortcuts;
  return [
    ...(search.project ?? []).map((name) => t.project(name)),
    search.state && t.summaryState(text.workBand.taskState[search.state]),
    search.needs && t.needsMe,
    search.includeArchived && t.includeArchived,
    search.q && t.search(search.q),
    search.by?.length && t.creators(search.by.join(', ')),
    search.sort && t.sort(text.pageList.sortOptions[search.sort]),
    search.stage && t.stage(search.stage),
    search.archived && t.archivedOnly,
  ].filter((value): value is string => typeof value === 'string');
}
