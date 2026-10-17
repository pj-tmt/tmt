import { describe, expect, it } from 'vite-plus/test';
import {
  dateLine,
  greetingPart,
  pagesChangedSince,
  RECENT_CHAPTERS,
  todayModel,
} from '../src/today-model.js';
import type { PageSummary } from '../src/transport.js';

const now = new Date(2026, 9, 17, 14, 30).getTime();
const hour = 3_600_000;
function page(id: string, ago: number | null, extra: Partial<PageSummary> = {}): PageSummary {
  return {
    id,
    title: id,
    sharing: 'private',
    lastUpdateAtMs: ago === null ? null : now - ago,
    ...extra,
  };
}
const waiting = { summary: { comments: 0, waiting: true, needsYou: 1 } };

describe('todayModel', () => {
  it('sums pending items rather than the number of waiting pages', () => {
    const pages = [
      page('three', hour, { summary: { comments: 0, waiting: true, needsYou: 3 } }),
      page('two', 2 * hour, { summary: { comments: 0, waiting: true, needsYou: 2 } }),
      page('archived', hour, {
        archived: true,
        summary: { comments: 0, waiting: true, needsYou: 9 },
      }),
    ];
    expect(todayModel(pages, now).counts.waiting).toBe(5);
  });

  it('puts waiting pages first, newest first, then the recent pages', () => {
    const model = todayModel(
      [
        page('old', 30 * hour),
        page('wait-old', 5 * hour, waiting),
        page('new', hour),
        page('wait-new', 2 * hour, waiting),
      ],
      now,
    );
    expect(model.waiting.map((p) => p.id)).toEqual(['wait-new', 'wait-old']);
    expect(model.recent.map((p) => p.id)).toEqual(['new', 'old']);
    expect(model.folded).toEqual([]);
  });

  it('keeps waiting pages out of the recent limit and folds the older rest', () => {
    const pages = Array.from({ length: RECENT_CHAPTERS + 3 }, (_, index) =>
      page(`p${index}`, (index + 1) * hour),
    );
    const model = todayModel([page('w', 40 * hour, waiting), ...pages], now);
    expect(model.waiting.map((p) => p.id)).toEqual(['w']);
    expect(model.recent).toHaveLength(RECENT_CHAPTERS);
    expect(model.folded.map((p) => p.id)).toEqual(['p10', 'p11', 'p12']);
  });

  it('leaves archived pages to All pages and never counts them', () => {
    const model = todayModel([page('a', hour, { archived: true }), page('b', hour)], now);
    expect(model.recent.map((p) => p.id)).toEqual(['b']);
    expect(model.counts).toEqual({ waiting: 0, updatedToday: 1, pages: 1, complete: false });
  });

  it('counts pages updated since the start of the viewer day', () => {
    const model = todayModel(
      [
        page('now', 0),
        page('earlier', 10 * hour),
        page('yesterday', 15 * hour),
        page('none', null),
      ],
      now,
    );
    expect(model.counts).toEqual({ waiting: 0, updatedToday: 2, pages: 4, complete: false });
  });

  it('keeps incomplete counts distinct from a confirmed zero and ignores unknown archived pages', () => {
    const unknown = page('unknown', hour);
    const known = page('known', hour, { summary: { comments: 0, waiting: true, needsYou: 3 } });
    expect(todayModel([unknown, known], now).counts).toMatchObject({ waiting: 3, complete: false });
    expect(todayModel([unknown], now).counts).toMatchObject({ waiting: 0, complete: false });
    expect(
      todayModel(
        [
          { ...unknown, archived: true },
          { ...known, summary: { comments: 0, waiting: false, needsYou: 0 } },
        ],
        now,
      ).counts,
    ).toMatchObject({ waiting: 0, complete: true });
  });

  it('counts a page with no summary as unknown and keeps pages without an update time last', () => {
    const model = todayModel([page('none', null), page('known', hour)], now);
    expect(model.counts.waiting).toBe(0);
    expect(model.recent.map((p) => p.id)).toEqual(['known', 'none']);
  });
});

describe('greeting and date', () => {
  it.each([
    [4, 'evening'],
    [5, 'morning'],
    [11, 'morning'],
    [12, 'afternoon'],
    [17, 'afternoon'],
    [18, 'evening'],
    [23, 'evening'],
  ] as const)('hour %i is %s', (hours, part) => {
    expect(greetingPart(new Date(2026, 9, 17, hours).getTime())).toBe(part);
  });

  it('writes the weekday, month and day', () => {
    expect(dateLine(now)).toBe('Saturday, October 17');
  });
});

it('uses admitted live counts without moving a completed chapter or claiming unknown pages are empty', () => {
  const first = page('first', hour, waiting);
  const unknown = page('unknown', 2 * hour);
  const counts = new Map([['first', 0]]);
  const model = todayModel([first, unknown], now, counts);
  expect(model.waiting).toEqual([first]);
  expect(model.counts).toMatchObject({ waiting: 0, complete: false });
  counts.set('unknown', 2);
  expect(todayModel([first, unknown], now, counts).counts).toMatchObject({
    waiting: 2,
    complete: true,
  });
});

describe('pagesChangedSince', () => {
  const summary = { lines: ['a', 'b'] as [string, string], atMs: now - 3 * hour };

  it('is true only when an active page changed after the summary was written', () => {
    expect(pagesChangedSince(summary, [page('newer', hour)])).toBe(true);
    expect(pagesChangedSince(summary, [page('older', 5 * hour)])).toBe(false);
    expect(pagesChangedSince(summary, [])).toBe(false);
  });

  it('does not count the moment of the write, an unknown update time or an archived page', () => {
    expect(pagesChangedSince(summary, [page('same', 3 * hour)])).toBe(false);
    expect(pagesChangedSince(summary, [page('unknown', null)])).toBe(false);
    expect(pagesChangedSince(summary, [page('hidden', hour, { archived: true })])).toBe(false);
  });
});
