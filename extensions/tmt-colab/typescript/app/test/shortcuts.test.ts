import { expect, it } from 'vite-plus/test';
import { validateHomeSearch, listPages } from '../src/page-list-filter.js';
import { shortcutCounts, shortcutSearch } from '../src/shortcuts.js';
import type { PageSummary } from '../src/transport.js';
const pages = [
  {
    id: 'active',
    title: 'Plan',
    sharing: 'private',
    state: 'todo',
    projects: ['design'],
    summary: { author: 'Ana', waiting: true },
  },
  {
    id: 'archived',
    title: 'Plan archived',
    sharing: 'private',
    state: 'done',
    archived: true,
    projects: ['design'],
  },
  { id: 'unknown', title: 'Plan unknown', sharing: 'private', projects: ['design'] },
] as PageSummary[];
it('manual state is independent of stage and missing state never matches todo', () => {
  expect(listPages(pages, validateHomeSearch({ state: 'todo' })).map((p) => p.id)).toEqual([
    'active',
  ]);
  expect(validateHomeSearch({ state: 'discussion' }).state).toBeUndefined();
});
it('include archived is a union; archived remains archived-only', () => {
  expect(listPages(pages, { includeArchived: true })).toHaveLength(3);
  expect(listPages(pages, { archived: true, includeArchived: true }).map((p) => p.id)).toEqual([
    'archived',
  ]);
  expect(listPages(pages, {})).toHaveLength(2);
});
it('saved filters retain other clauses and normalize project scope', () => {
  expect(
    shortcutSearch({
      q: 'Plan',
      by: ['Ana'],
      sort: 'title',
      stage: 'done',
      scope: 'design',
      project: ['review'],
      state: 'done',
      includeArchived: true,
    }),
  ).toEqual({
    q: 'Plan',
    by: ['Ana'],
    sort: 'title',
    stage: 'done',
    project: ['design', 'review'],
    state: 'done',
    includeArchived: true,
  });
});
it('one match applies projects, text, creator, manual state and needs together', () => {
  expect(
    listPages(pages, { project: ['design'], q: 'Plan', by: ['Ana'], state: 'todo', needs: true }),
  ).toHaveLength(1);
  expect(listPages(pages, { project: ['design', 'review'] })).toHaveLength(0);
});

it('archived and needs-you tabs keep their existing precedence over a retained stage parameter', () => {
  expect(listPages(pages, { archived: true, stage: 'done' }).map((page) => page.id)).toEqual([
    'archived',
  ]);
  expect(listPages(pages, { needs: true, stage: 'done' }).map((page) => page.id)).toEqual([
    'active',
  ]);
});

it('shortcut counts remain unknown until the data needed by each filter is available', () => {
  expect(shortcutCounts(undefined, {})).toEqual({});
  expect(shortcutCounts(pages, {})).toEqual({ total: 2 });
  expect(shortcutCounts(pages, { stage: 'all' })).toEqual({ total: 2 });
  expect(shortcutCounts(pages, { needs: true })).toEqual({});
  expect(shortcutCounts(pages, { q: 'Plan' })).toEqual({});
  expect(
    shortcutCounts([{ ...pages[0]!, stateError: 'page-unavailable' }], { state: 'todo' }),
  ).toEqual({});
  expect(shortcutCounts(pages, { project: ['other'], needs: true })).toEqual({
    total: 0,
    attention: 0,
  });
  const complete = pages.map((page) =>
    page.summary ? page : { ...page, summary: { comments: 0, needsYou: 0, waiting: false } },
  );
  expect(shortcutCounts(complete, { project: ['design'] })).toEqual({ total: 2, attention: 1 });
  expect(shortcutCounts(complete, { needs: true })).toEqual({ total: 1, attention: 1 });
});
