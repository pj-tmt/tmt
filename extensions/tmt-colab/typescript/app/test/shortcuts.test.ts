import { expect, it } from 'vite-plus/test';
import { validateHomeSearch, listPages } from '../src/page-list-filter.js';
import { shortcutSearch } from '../src/shortcuts.js';
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
