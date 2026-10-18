import { expect, it } from 'vite-plus/test';
import type { Shell } from '../src/shell.js';
import { pagesChangedSince, shellByline, shellCells } from '../src/shell-grid.js';
import type { PageSummary } from '../src/transport.js';

const A = '10000000-0000-4000-8000-00000000000a';
const B = '10000000-0000-4000-8000-00000000000b';
const GONE = '10000000-0000-4000-8000-0000000000ff';
const id = (n: number) => `20000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
const page = (pageId: string, extra: Partial<PageSummary> = {}): PageSummary => ({
  id: pageId,
  title: `Page ${pageId.slice(-1)}`,
  sharing: 'private',
  ...extra,
});
const attribution = { writerId: '30000000-0000-4000-8000-000000000001', timeMs: 1_000 };
const shell: Shell = {
  version: 1,
  items: {
    [id(1)]: { kind: 'page', pageId: A, note: 'Start here', audience: 'Reviewers' },
    [id(2)]: { kind: 'anchor', pageId: B, anchor: 'intro' },
    [id(3)]: { kind: 'reading', pageId: A, readingId: '40000000-0000-4000-8000-000000000001' },
    [id(4)]: { kind: 'page', pageId: GONE },
  },
  order: [id(2), id(1), id(4), id(3)],
};

it('lists the items in order, each matched to its source page, and keeps the same page twice', () => {
  const cells = shellCells(shell, [
    page(A, { summary: { comments: 2, waiting: false, needsYou: 3 } }),
    page(B, { archived: true }),
  ]);
  expect(cells.map((cell) => cell.id)).toEqual([id(2), id(1), id(4), id(3)]);
  expect(cells[0]).toMatchObject({
    kind: 'anchor',
    anchor: 'intro',
    title: 'Page b',
    archived: true,
  });
  expect(cells[1]).toMatchObject({
    kind: 'page',
    title: 'Page a',
    note: 'Start here',
    needsYou: 3,
  });
  expect(cells[1].audience).toBe('Reviewers');
  expect(cells[3]).toMatchObject({ kind: 'reading', pageId: A, title: 'Page a', needsYou: 3 });
  expect(cells[3].readingId).toBe('40000000-0000-4000-8000-000000000001');
});

it('keeps an item whose page the space does not show as unavailable and grants nothing', () => {
  const [, , gone] = shellCells(shell, [page(A), page(B)]);
  expect(gone).toMatchObject({ pageId: GONE, unavailable: true, title: '', needsYou: 0 });
  expect(shellCells(shell, []).every((cell) => cell.unavailable)).toBe(true);
});

it('reports a title the browser has not cached yet and omits absent optional fields', () => {
  const [first, second] = shellCells(shell, [page(B, { title: '', titleMissing: true }), page(A)]);
  expect(first.titleMissing).toBe(true);
  expect(second.titleMissing).toBe(false);
  expect('note' in first).toBe(false);
  expect('anchor' in second).toBe(false);
});

it('names the summary writer only by a Core-resolved display agent', () => {
  expect(shellByline(shell)).toBeUndefined();
  const withSummary = (extra: object): Shell => ({
    ...shell,
    summary: { line1: 'One', line2: 'Two', attribution: { ...attribution, ...extra } },
  });
  expect(shellByline(withSummary({}))).toEqual({ timeMs: 1_000 });
  expect(shellByline(withSummary({ displayAgent: { name: 'Atlas' } }))).toEqual({
    who: 'Atlas',
    timeMs: 1_000,
  });
});

it('counts the distinct pages edited after the summary, ignoring unknown times and missing pages', () => {
  const cells = shellCells(shell, [
    page(A, { lastUpdateAtMs: 2_000 }),
    page(B, { lastUpdateAtMs: null }),
  ]);
  // A is named by two items but counts once; B has no known edit time; GONE is unavailable.
  expect(pagesChangedSince(cells, 1_000)).toBe(1);
  // Same owner clock: an edit in the very millisecond of the summary is not after it.
  expect(pagesChangedSince(cells, 2_000)).toBe(0);
  expect(pagesChangedSince(cells, 1_999)).toBe(1);
  expect(pagesChangedSince([], 0)).toBe(0);
});
