import { expect, it } from 'vite-plus/test';
import type { PageSummary } from '../src/transport.js';
import type { Work } from '../src/work.js';
import { subpageTree } from '../src/work-tree.js';

const row = (
  id: string,
  state: PageSummary['state'],
  parent?: { page: string; task: string },
): PageSummary => ({
  id,
  title: id.toUpperCase(),
  state,
  sharing: 'private',
  ...(parent ? { summary: { comments: 0, waiting: false, needsYou: 0, parent } } : {}),
});
const work: Work = {
  version: 1,
  next: 5,
  order: ['T-1', 'T-2', 'T-3', 'T-4'],
  tasks: {
    'T-1': { title: 'b', state: 'todo', child: 'b' },
    'T-2': { title: 'plain', state: 'done' },
    'T-3': { title: 'c', state: 'todo', child: 'c' },
    // Two tasks following one page (a writers' race) still draw one row.
    'T-4': { title: 'b again', state: 'todo', child: 'b' },
  },
};
const pages = [
  row('top', 'todo'),
  row('a', 'in-progress', { page: 'top', task: 'T-1' }),
  row('b', 'done', { page: 'a', task: 'T-1' }),
  row('c', 'in-progress', { page: 'a', task: 'T-3' }),
  row('c2', 'done', { page: 'c', task: 'T-9' }),
  row('c1', 'todo', { page: 'c', task: 'T-2' }),
  row('free', 'todo'),
  row('elsewhere', 'todo', { page: 'top', task: 'T-2' }),
];

it('lists direct sub-pages in task order and deeper levels in parent task order', () => {
  const tree = subpageTree('a', work, pages);
  expect(tree.top.map(({ id, carrier, parent }) => [id, carrier, parent])).toEqual([
    ['b', 'T-1', 'a'],
    ['c', 'T-3', 'a'],
  ]);
  // T-2 before T-9, numerically.
  expect(tree.childrenOf('c').map(({ id, carrier }) => [id, carrier])).toEqual([
    ['c1', 'T-2'],
    ['c2', 'T-9'],
  ]);
  expect(tree.childrenOf('a')).toBe(tree.top);
  expect(tree.childrenOf('free')).toEqual([]);
});

it('counts only direct sub-pages by their own state, unknown when the list has none', () => {
  const tree = subpageTree('a', work, pages);
  expect(tree.counts(tree.top)).toEqual({ done: 1, inProgress: 1, todo: 0, unknown: 0, total: 2 });
  const unlisted = subpageTree('a', work, undefined);
  expect(unlisted.top.map((page) => page.title)).toEqual([undefined, undefined]);
  expect(unlisted.counts(unlisted.top)).toMatchObject({ unknown: 2, total: 2 });
});

it('names why a page cannot be attached: itself, above it, or under another page', () => {
  const tree = subpageTree('a', work, pages);
  expect(tree.refusal('a')).toEqual({ kind: 'self' });
  expect(tree.refusal('top')).toEqual({ kind: 'loop' });
  expect(tree.refusal('b')).toEqual({ kind: 'attached', parentTitle: 'A' });
  expect(tree.refusal('elsewhere')).toEqual({ kind: 'attached', parentTitle: 'TOP' });
  expect(tree.refusal('free')).toBeUndefined();
});

it('ends the ancestor walk on a stored loop', () => {
  const looped = [
    row('x', 'todo', { page: 'y', task: 'T-1' }),
    row('y', 'todo', { page: 'x', task: 'T-1' }),
  ];
  expect(subpageTree('x', undefined, looped).refusal('y')).toEqual({ kind: 'loop' });
});
