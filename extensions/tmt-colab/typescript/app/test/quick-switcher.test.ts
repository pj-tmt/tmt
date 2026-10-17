import { expect, it } from 'vite-plus/test';
import type { PageFilter } from '../src/labels.js';
import { SWITCHER_RESULTS, rankPages } from '../src/quick-switcher.js';
import type { PageSummary } from '../src/transport.js';

const id = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
const page = (
  n: number,
  title: string,
  extra: {
    labels?: string[];
    author?: string;
    waiting?: boolean;
    needsYou?: number;
    archived?: boolean;
    updated?: number;
  } = {},
): PageSummary => ({
  id: id(n),
  title,
  sharing: 'private',
  ...(extra.archived ? { archived: true } : {}),
  ...(extra.labels ? { labels: extra.labels } : {}),
  ...(extra.updated === undefined ? {} : { lastUpdateAtMs: extra.updated }),
  ...(extra.author !== undefined || extra.waiting !== undefined
    ? {
        summary: {
          comments: 0,
          waiting: extra.waiting ?? false,
          needsYou: extra.needsYou ?? Number(extra.waiting ?? false),
          ...(extra.author === undefined ? {} : { author: extra.author }),
        },
      }
    : {}),
});
const any: PageFilter = { labels: [], authors: [], text: '' };
const none = new Map<string, number>();
const titles = (pages: readonly PageSummary[], query: Partial<PageFilter> = {}, opened = none) =>
  rankPages(pages, { ...any, ...query }, opened).map((result) => result.page.title);

it('without text, a page waiting for you comes first, then the most recently opened or updated', () => {
  const pages = [
    page(1, 'Old', { updated: 100 }),
    page(2, 'Newest edit', { updated: 900 }),
    page(3, 'Opened lately', { updated: 60 }),
    page(4, 'Waiting', { updated: 10, waiting: true, needsYou: 1 }),
    page(5, 'Never touched'),
  ];
  const opened = new Map([[id(3), 800]]);
  expect(titles(pages, {}, opened)).toEqual([
    'Waiting',
    'Newest edit',
    'Opened lately',
    'Old',
    'Never touched',
  ]);
  // Recency is the later of the two: opening an old page does not push it past a newer edit,
  // and an edit newer than the last open counts.
  expect(titles(pages, {}, new Map([[id(1), 500]]))).toEqual([
    'Waiting',
    'Newest edit',
    'Old',
    'Opened lately',
    'Never touched',
  ]);
  expect(titles(pages, {}, new Map([[id(1), 50]]))).toEqual([
    'Waiting',
    'Newest edit',
    'Old',
    'Opened lately',
    'Never touched',
  ]);
});

it('ties break by title, then ID', () => {
  const pages = [
    page(3, 'beta', { updated: 5 }),
    page(2, 'Alpha', { updated: 5 }),
    page(1, 'alpha', { updated: 5 }),
    page(4, 'Gamma'),
    page(5, 'Delta'),
  ];
  const ranked = rankPages(pages, any, none).map((result) => result.page.id);
  expect(ranked).toEqual([id(1), id(2), id(3), id(5), id(4)]);
});

it('free text ranks by match strength first, whatever the recency', () => {
  const pages = [
    page(1, 'Quarterly review', { updated: 1, author: 'Plan Bot' }),
    page(2, 'Notes', { updated: 2, labels: ['plan'] }),
    page(3, 'The big plan', { updated: 3 }),
    page(4, 'Replan', { updated: 4 }),
    page(5, 'Plan: kickoff', { updated: 5 }),
    page(6, 'Planning', { updated: 6 }),
  ];
  const ranked = rankPages(pages, { ...any, text: 'plan' }, none);
  expect(ranked.map((result) => [result.page.title, result.matched])).toEqual([
    ['Planning', 'title'],
    ['Plan: kickoff', 'title'],
    ['The big plan', 'title'],
    ['Replan', 'title'],
    ['Notes', 'label'],
    ['Quarterly review', 'author'],
  ]);
});

it('within one strength, waiting comes before recency; no free text reports no match', () => {
  const pages = [
    page(1, 'Plan A', { updated: 900 }),
    page(2, 'Plan B', { updated: 100, waiting: true, needsYou: 1 }),
    page(3, 'Plan C', { updated: 500 }),
  ];
  expect(titles(pages, { text: 'plan' })).toEqual(['Plan B', 'Plan A', 'Plan C']);
  expect(rankPages(pages, any, none).every((result) => !('matched' in result))).toBe(true);
});

it('a multi-word query is as weak as its weakest word, and every word must match', () => {
  const pages = [
    page(1, 'Launch plan', { labels: ['q4'], updated: 1 }),
    page(2, 'Launch notes', { labels: ['q4 plan'], updated: 2 }),
    page(3, 'Launch', { updated: 3 }),
  ];
  const ranked = rankPages(pages, { ...any, text: 'launch q4' }, none);
  expect(ranked.map((result) => [result.page.title, result.matched])).toEqual([
    ['Launch notes', 'label'],
    ['Launch plan', 'label'],
  ]);
  expect(titles(pages, { text: 'launch nothing' })).toEqual([]);
  // Case and surrounding whitespace never matter.
  expect(titles(pages, { text: '  LAUNCH  ' })).toEqual(['Launch', 'Launch notes', 'Launch plan']);
});

it('the list filter applies as on the list: labels all, creators any, archived never', () => {
  const pages = [
    page(1, 'A', { labels: ['design', 'ops'], author: 'Ada', updated: 4 }),
    page(2, 'B', { labels: ['design'], author: 'Studio agent', updated: 3 }),
    page(3, 'C', { labels: ['design'], updated: 2 }),
    page(4, 'D', { labels: ['design', 'ops'], author: 'Ada', updated: 1, archived: true }),
  ];
  expect(titles(pages, { labels: ['design'] })).toEqual(['A', 'B', 'C']);
  expect(titles(pages, { labels: ['design', 'ops'] })).toEqual(['A']);
  expect(titles(pages, { authors: ['Ada', 'Studio agent'] })).toEqual(['A', 'B']);
  expect(titles(pages, { labels: ['design'], authors: ['Studio agent'] })).toEqual(['B']);
  expect(titles(pages, { text: 'ada' })).toEqual(['A']);
  expect(titles(pages, { authors: ['Nobody'] })).toEqual([]);
});

it('exclude removes one page before the limit, and the limit bounds the results', () => {
  const pages = Array.from({ length: 60 }, (_, index) =>
    page(index + 1, `Page ${String(index + 1).padStart(2, '0')}`, { updated: 1000 - index }),
  );
  expect(rankPages(pages, any, none)).toHaveLength(SWITCHER_RESULTS);
  const first = rankPages(pages, any, none)[0].page;
  const without = rankPages(pages, any, none, { exclude: first.id, limit: 2 });
  expect(without.map((result) => result.page.title)).toEqual(['Page 02', 'Page 03']);
  expect(rankPages(pages, any, none, { limit: 0 })).toEqual([]);
  expect(rankPages(pages, any, none, { limit: 3.9 })).toHaveLength(3);
  expect(rankPages(pages, any, none, { exclude: id(999) })).toHaveLength(SWITCHER_RESULTS);
});

it('a title word matches by its prefix across punctuation and non-Latin letters, and the input is not changed', () => {
  const pages = [page(1, 'Q4-plan_review', { updated: 1 }), page(2, '计划 notes', { updated: 2 })];
  const before = JSON.stringify(pages);
  expect(titles(pages, { text: 'review' })).toEqual(['Q4-plan_review']);
  expect(titles(pages, { text: '计划' })).toEqual(['计划 notes']);
  expect(titles(pages, { text: 'lan' })).toEqual(['Q4-plan_review']);
  expect(JSON.stringify(pages)).toBe(before);
});

it('a page with no title is found by the text the list shows for it', () => {
  const pages = [page(1, '   ', { updated: 1 })];
  expect(rankPages(pages, { ...any, text: 'untitled' }, none)).toHaveLength(1);
});
