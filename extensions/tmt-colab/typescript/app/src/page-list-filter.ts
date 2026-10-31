import { AUTHOR_BYTES } from './bootstrap.js';
import { DEFAULT_VIEW, HOME_STAGES, type HomeStage } from './list-stage.js';
import { filterPages, projectNameError, type PageFilter } from './projects.js';
import type { PageSummary } from './transport.js';
import type { PageState } from './bootstrap.js';

/** The query that may filter and order the list (and a copied URL), bounded and validated. */
export interface HomeSearch {
  q?: string;
  project?: string[];
  /** The project the list is scoped to: the base the filters narrow; absent is All pages. */
  scope?: string;
  by?: string[];
  sort?: 'title';
  /** The tab: absent is `DEFAULT_VIEW`; `archived` takes precedence. */
  stage?: HomeStage;
  archived?: true;
  /** Only the pages with an item waiting on the viewer. */
  needs?: true;
  state?: PageState | PageState[];
  includeArchived?: true;
  shortcut?: string;
}
/** The route merges what this returns over the raw query, so every key is named here: one it
 * cannot read is set to `undefined` and replaces the raw value instead of reaching the list. */
export function validateHomeSearch(search: Record<string, unknown>): HomeSearch {
  const out: HomeSearch = {
    q: undefined,
    project: undefined,
    scope: undefined,
    by: undefined,
    sort: undefined,
    stage: undefined,
    archived: undefined,
    needs: undefined,
    state: undefined,
    includeArchived: undefined,
    shortcut: undefined,
  };
  if (typeof search.q === 'string' && search.q.trim()) out.q = search.q.trim().slice(0, 256);
  if (Array.isArray(search.project)) {
    const names = [
      ...new Set(
        search.project.filter(
          (name): name is string =>
            typeof name === 'string' && projectNameError(name) === undefined,
        ),
      ),
    ].slice(0, 8);
    if (names.length) out.project = names;
  }
  if (typeof search.scope === 'string' && projectNameError(search.scope) === undefined)
    out.scope = search.scope;
  if (Array.isArray(search.by)) {
    const names = [
      ...new Set(
        search.by.filter(
          (name): name is string =>
            typeof name === 'string' &&
            name.length > 0 &&
            new TextEncoder().encode(name).length <= AUTHOR_BYTES &&
            !/\p{Cc}/u.test(name),
        ),
      ),
    ].slice(0, 8);
    if (names.length) out.by = names;
  }
  if (search.sort === 'title') out.sort = 'title';
  // The default is the absence of the key, so only another stage is a value worth keeping.
  const stage = HOME_STAGES.find((name) => name === search.stage);
  if (stage && stage !== DEFAULT_VIEW) out.stage = stage;
  if (search.archived === true || search.archived === 'true') out.archived = true;
  if (search.needs === true || search.needs === 'true') out.needs = true;
  const states = Array.isArray(search.state) ? search.state : [search.state];
  const selected = [
    ...new Set(
      states.filter((state): state is PageState =>
        ['todo', 'in-progress', 'done'].includes(state as string),
      ),
    ),
  ];
  if (selected.length) out.state = selected;
  if (search.includeArchived === true || search.includeArchived === 'true')
    out.includeArchived = true;
  if (
    typeof search.shortcut === 'string' &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(search.shortcut)
  )
    out.shortcut = search.shortcut;
  return out;
}

/** The shared conditions for the list, shortcut counts and preview. */
export function pageListFilter(search: HomeSearch): PageFilter {
  return {
    projects: [...new Set([...(search.scope ? [search.scope] : []), ...(search.project ?? [])])],
    authors: search.by ?? [],
    text: search.q ?? '',
    needsYou: search.needs === true,
    state: search.state,
    ...(!search.archived && !search.needs && search.stage && search.stage !== 'all'
      ? { stage: search.stage }
      : {}),
  };
}
export function listPages(pages: readonly PageSummary[], search: HomeSearch): PageSummary[] {
  return filterPages(
    pages.filter((page) =>
      search.archived ? !!page.archived : search.includeArchived || !page.archived,
    ),
    pageListFilter(search),
    search.sort ?? 'updated',
  );
}
