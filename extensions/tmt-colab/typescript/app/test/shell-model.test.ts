import { describe, expect, it } from 'vite-plus/test';
import type { ReadingDetails } from '../src/reading-details.js';
import type { Shell } from '../src/shell.js';
import { shellCells } from '../src/shell-grid.js';
import {
  IN_PROGRESS_MS,
  detailPageIds,
  inProgress,
  shellModel,
  spellSmallNumber,
  startOfDay,
  timeOfDay,
} from '../src/shell-model.js';
import type { PageSummary } from '../src/transport.js';

const DAY = 86_400_000;
// A local-time anchor: 2026-10-21 10:30 in the machine's zone, so day arithmetic is zone-proof.
const NOW = new Date(2026, 9, 21, 10, 30).getTime();
const TODAY = startOfDay(NOW);
const pid = (n: number) => `10000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
const iid = (n: number) => `20000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
const READING = '40000000-0000-4000-8000-000000000001';

const page = (n: number, extra: Partial<PageSummary> = {}): PageSummary => ({
  id: pid(n),
  title: `Page ${n}`,
  sharing: 'private',
  lastUpdateAtMs: NOW - DAY,
  ...extra,
});
const waiting = (needsYou: number, author?: string) => ({
  comments: 0,
  waiting: needsYou > 0,
  needsYou,
  ...(author === undefined ? {} : { author }),
});
function shellOf(items: Shell['items'], order = Object.keys(items)): Shell {
  return { version: 1, items, order };
}
function details(
  n: number,
  needs: { kind: 'review' | 'question' | 'reading' | 'proposal'; id: string; title: string }[],
  latest?: { atMs: number; text: string },
): [string, ReadingDetails] {
  return [
    pid(n),
    {
      pageId: pid(n),
      state: 'ready',
      fingerprint: 'a'.repeat(64),
      summary: { comments: 0, waiting: needs.length > 0, needsYou: needs.length },
      readings: [],
      needs,
      ...(latest ? { latest } : {}),
    },
  ];
}
const model = (
  items: Shell['items'],
  pages: PageSummary[],
  info: [string, ReadingDetails][] = [],
  order?: string[],
) => shellModel(shellCells(shellOf(items, order), pages), new Map(info), NOW);

describe('the greeting and the count row', () => {
  it('uses the local time of day and spells one to nine out', () => {
    const at = (hour: number) => new Date(2026, 9, 21, hour, 0).getTime();
    expect([4, 5, 11, 12, 17, 18, 23].map((hour) => timeOfDay(at(hour)))).toEqual([
      'evening',
      'morning',
      'morning',
      'afternoon',
      'afternoon',
      'evening',
      'evening',
    ]);
    expect([0, 1, 3, 9, 10, 12].map(spellSmallNumber)).toEqual([
      '0',
      'one',
      'three',
      'nine',
      '10',
      '12',
    ]);
  });

  it('counts what waits, what is in progress and what changed today over distinct pages', () => {
    const m = model(
      {
        [iid(1)]: { kind: 'page', pageId: pid(1) },
        [iid(2)]: { kind: 'reading', pageId: pid(1), readingId: READING },
        [iid(3)]: { kind: 'page', pageId: pid(2) },
        [iid(4)]: { kind: 'page', pageId: pid(3) },
        [iid(5)]: { kind: 'page', pageId: pid(4) },
        [iid(6)]: { kind: 'page', pageId: pid(5) },
      },
      [
        page(1, { summary: waiting(3), lastUpdateAtMs: TODAY + 1000 }),
        page(2, { summary: waiting(2) }),
        page(3, { lastUpdateAtMs: TODAY + 5 }),
        page(4, { lastUpdateAtMs: NOW - 3 * DAY }),
        page(5, { lastUpdateAtMs: NOW - 30 * DAY }),
      ],
    );
    // Page 1 is named twice and counts once: 3 + 2 waiting, and the other three are not waiting.
    expect(m.counts).toEqual({ waiting: 5, inProgress: 2, updatedToday: 2 });
    expect(m.waiting).toBe(5);
    expect(m.chapters.map((c) => [c.pageId, c.stage])).toEqual([
      [pid(1), 'waiting'],
      [pid(2), 'waiting'],
      [pid(3), 'in-progress'],
      [pid(4), 'in-progress'],
    ]);
    expect(m.earlier.map((row) => row.pageId)).toEqual([pid(5)]);
  });

  it('says nothing is waiting for an empty or settled shell', () => {
    expect(model({}, []).counts).toEqual({ waiting: 0, inProgress: 0, updatedToday: 0 });
    const settled = model({ [iid(1)]: { kind: 'page', pageId: pid(1) } }, [page(1)]);
    expect(settled.waiting).toBe(0);
  });

  it('never counts an archived or unavailable reference, and lists the unavailable one', () => {
    const m = model(
      {
        [iid(1)]: { kind: 'page', pageId: pid(1) },
        [iid(2)]: { kind: 'page', pageId: pid(2) },
        [iid(3)]: { kind: 'page', pageId: pid(9) },
      },
      [
        page(1, { archived: true, summary: waiting(4) }),
        page(2, { summary: waiting(1), lastUpdateAtMs: TODAY + 1 }),
      ],
    );
    expect(m.counts).toEqual({ waiting: 1, inProgress: 0, updatedToday: 1 });
    expect(m.chapters.map((c) => c.pageId)).toEqual([pid(2)]);
    expect(m.unavailable).toEqual([pid(9)]);
  });
});

describe('the order of the chapters', () => {
  it('puts waiting first, then in progress, each by update time, with the shell order as tie-break', () => {
    const m = model(
      {
        [iid(1)]: { kind: 'page', pageId: pid(1) },
        [iid(2)]: { kind: 'page', pageId: pid(2) },
        [iid(3)]: { kind: 'page', pageId: pid(3) },
        [iid(4)]: { kind: 'page', pageId: pid(4) },
        [iid(5)]: { kind: 'page', pageId: pid(5) },
      },
      [
        page(1, { lastUpdateAtMs: NOW - 2 * DAY }),
        page(2, { summary: waiting(1), lastUpdateAtMs: NOW - 5 * DAY }),
        page(3, { summary: waiting(1), lastUpdateAtMs: NOW - 5 * DAY }),
        page(4, { lastUpdateAtMs: NOW - 1 * DAY }),
        page(5, { summary: waiting(2), lastUpdateAtMs: NOW - 1 * DAY }),
      ],
      [],
      [iid(3), iid(2), iid(1), iid(4), iid(5)],
    );
    // Waiting by time (5 is newest), the 2/3 tie broken by the shell order (3 before 2), then
    // in progress by time (4 before 1). Chapters number from one in that order.
    expect(m.chapters.map((c) => c.pageId)).toEqual([pid(5), pid(3), pid(2), pid(4), pid(1)]);
    expect(m.chapters.map((c) => c.number)).toEqual([1, 2, 3, 4, 5]);
  });

  it('folds pages older than seven days, and pages with no known edit, into the earlier list', () => {
    const m = model(
      {
        [iid(1)]: { kind: 'page', pageId: pid(1) },
        [iid(2)]: { kind: 'page', pageId: pid(2) },
        [iid(3)]: { kind: 'page', pageId: pid(3) },
      },
      [
        page(1, { lastUpdateAtMs: NOW - IN_PROGRESS_MS }),
        page(2, { lastUpdateAtMs: NOW - IN_PROGRESS_MS - 1 }),
        page(3, { lastUpdateAtMs: null }),
      ],
    );
    expect(m.chapters.map((c) => c.pageId)).toEqual([pid(1)]);
    expect(m.earlier.map((r) => r.pageId)).toEqual([pid(2), pid(3)]);
  });

  it('decides in progress in one place that a page work stage can replace', () => {
    expect(inProgress({ lastUpdateAtMs: NOW }, NOW)).toBe(true);
    expect(inProgress({ lastUpdateAtMs: null }, NOW)).toBe(false);
  });
});

describe('the passage of a chapter', () => {
  const items = (extra: Shell['items'] = {}): Shell['items'] => ({
    [iid(1)]: { kind: 'page', pageId: pid(1), note: 'Start here' },
    ...extra,
  });
  const waitingPage = [page(1, { summary: waiting(2, 'tmt-feat-lead'), projects: ['Design'] })];
  const needs = [
    { kind: 'review' as const, id: pid(100), title: 'Review the plan' },
    { kind: 'reading' as const, id: READING, title: 'Read the notes' },
  ];

  it('carries the title, author, label and time of the page on the left', () => {
    const [chapter] = model(items(), waitingPage).chapters;
    expect(chapter).toMatchObject({
      number: 1,
      title: 'Page 1',
      author: 'tmt-feat-lead',
      label: 'Design',
      needsYou: 2,
      itemIds: [iid(1)],
    });
  });

  it('shows the first pending item by rank and how many more wait', () => {
    const [chapter] = model(items(), waitingPage, [details(1, needs)]).chapters;
    expect(chapter.passage).toEqual({ kind: 'need', need: needs[0], more: 1 });
  });

  it('puts the target an item names first, by reading ID or by anchor', () => {
    const reading = model(
      items({ [iid(2)]: { kind: 'reading', pageId: pid(1), readingId: READING } }),
      waitingPage,
      [details(1, needs)],
    ).chapters[0];
    expect(reading.passage).toMatchObject({ kind: 'need', need: { id: READING } });
    const anchor = model(
      items({ [iid(2)]: { kind: 'anchor', pageId: pid(1), anchor: pid(100) } }),
      waitingPage,
      [details(1, [needs[1], needs[0]])],
    ).chapters[0];
    expect(anchor.passage).toMatchObject({ kind: 'need', need: { id: pid(100) } });
    expect(reading.itemIds).toEqual([iid(1), iid(2)]);
  });

  it('falls back to the latest comment, then the item note, and waits while details are missing', () => {
    const latest = { atMs: NOW - 1000, text: 'A reply' };
    expect(model(items(), [page(1)], [details(1, [], latest)]).chapters[0].passage).toEqual({
      kind: 'latest',
      latest,
    });
    expect(model(items(), [page(1)], [details(1, [])]).chapters[0].passage).toEqual({
      kind: 'note',
      note: 'Start here',
    });
    expect(
      model({ [iid(1)]: { kind: 'page', pageId: pid(1) } }, [page(1)]).chapters[0].passage,
    ).toEqual({ kind: 'none' });
    // A waiting page whose details have not arrived yet is pending, not an empty chapter.
    expect(model(items(), waitingPage).chapters[0].passage).toEqual({ kind: 'pending' });
    // A pending or unavailable answer is not ready: the item note stands, never a false passage.
    expect(
      model(items(), waitingPage, [[pid(1), { pageId: pid(1), state: 'unavailable' }]]).chapters[0]
        .passage,
    ).toEqual({ kind: 'note', note: 'Start here' });
    expect(
      model({ [iid(1)]: { kind: 'page', pageId: pid(1) } }, waitingPage, [
        [pid(1), { pageId: pid(1), state: 'pending' }],
      ]).chapters[0].passage,
    ).toEqual({ kind: 'none' });
  });

  it('asks for the details of the first twenty chapters only', () => {
    const many: Shell['items'] = {};
    const pages: PageSummary[] = [];
    for (let n = 1; n <= 30; n++) {
      many[iid(n)] = { kind: 'page', pageId: pid(n) };
      pages.push(page(n, { summary: waiting(1), lastUpdateAtMs: NOW - n }));
    }
    const m = model(many, pages);
    expect(detailPageIds(m)).toEqual(Array.from({ length: 20 }, (_, i) => pid(i + 1)));
  });
});

describe('the element an anchor item names', () => {
  it('rides on the chapter so its passage opens the page there', () => {
    const [chapter] = model(
      {
        [iid(1)]: { kind: 'page', pageId: pid(1) },
        [iid(2)]: { kind: 'anchor', pageId: pid(1), anchor: 'decisions' },
        [iid(3)]: { kind: 'anchor', pageId: pid(1), anchor: 'later' },
      },
      [page(1)],
    ).chapters;
    expect(chapter.anchor).toBe('decisions');
    expect(
      model({ [iid(1)]: { kind: 'page', pageId: pid(1) } }, [page(1)]).chapters[0],
    ).not.toHaveProperty('anchor');
  });
});
