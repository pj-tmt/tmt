import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import {
  applyLabelChanges,
  compareLabels,
  filterPages,
  authorCounts,
  labelCounts,
  labelNameError,
  labelRequests,
  relabel,
  withLabel,
  waitingCount,
  type LabelChange,
  type PageFilter,
} from '../src/labels.js';
import type { PageSummary } from '../src/transport.js';

const id = (n: number) => `10000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
function page(n: number, labels: string[], changes: Partial<PageSummary> = {}): PageSummary {
  return {
    id: id(n),
    title: `Page ${n}`,
    sharing: 'private',
    lastUpdateAtMs: 1000 - n,
    projects: labels,
    ...changes,
  };
}

describe('label names', () => {
  it('follow the owner store grammar', () => {
    for (const name of ['a', 'Q4 plan', '日本語', '🚀 launch', 'x'.repeat(32)])
      expect(labelNameError(name), name).toBeUndefined();
    expect(labelNameError('')).toBe('empty');
    expect(labelNameError('x'.repeat(33))).toBe('long');
    // 32 astral characters are 32 points, not 64 UTF-16 units.
    expect(labelNameError('🚀'.repeat(32))).toBeUndefined();
    for (const name of [' a', 'a ', 'a  b', 'a\tb', 'a\nb', 'a b', 'a\u0000b', '　'])
      expect(labelNameError(name), JSON.stringify(name)).toBe('form');
  });
  // The owner store reads the same file (projects.rs): the two grammars cannot drift. U+FEFF and
  // U+200B are not White_Space, so a CLI label carrying one must stay editable here.
  it('agree with the shared vector the owner store reads', () => {
    const vector = JSON.parse(
      readFileSync(
        new URL('../../../contracts/vectors/project-name-v1.json', import.meta.url),
        'utf8',
      ),
    ) as { valid: string[]; invalid: string[] };
    expect(vector.valid.length).toBeGreaterThan(0);
    expect(vector.invalid.length).toBeGreaterThan(0);
    for (const name of vector.valid)
      expect(labelNameError(name), JSON.stringify(name)).toBeUndefined();
    for (const name of vector.invalid)
      expect(labelNameError(name), JSON.stringify(name)).toBeDefined();
    expect(labelNameError('a\ufeffb')).toBeUndefined();
  });
  it('sort by code point like the owner store, not by UTF-16 unit', () => {
    // U+FF5E sorts before U+1F680 by code point, after it by UTF-16 unit.
    expect(compareLabels('～', '🚀')).toBeLessThan(0);
    expect(['b', 'a', 'B', '日'].sort(compareLabels)).toEqual(['B', 'a', 'b', '日']);
    expect(compareLabels('a', 'ab')).toBeLessThan(0);
  });
});

describe('the page list', () => {
  const pages = [
    page(1, ['design', 'q4 plan']),
    page(2, ['design']),
    page(3, [], { title: 'Design notes' }),
    page(4, ['ops'], { archived: true }),
  ];
  it('counts each label over the pages it is given', () => {
    expect(labelCounts(pages)).toEqual([
      { name: 'design', count: 2 },
      { name: 'ops', count: 1 },
      { name: 'q4 plan', count: 1 },
    ]);
    expect(labelCounts(pages.filter((p) => !p.archived))).toHaveLength(2);
  });
  it('requires every label token and every word in the title or a label', () => {
    const ids = (labels: string[], text: string) =>
      filterPages(pages, { labels, authors: [], text }, 'updated').map((p) => p.id);
    expect(ids([], '')).toEqual([id(1), id(2), id(3), id(4)]);
    expect(ids(['design'], '')).toEqual([id(1), id(2)]);
    expect(ids(['design', 'q4 plan'], '')).toEqual([id(1)]);
    expect(ids([], 'DESIGN')).toEqual([id(1), id(2), id(3)]);
    expect(ids([], 'design notes')).toEqual([id(3)]);
    expect(ids([], 'q4 page')).toEqual([id(1)]);
    expect(ids(['missing'], '')).toEqual([]);
  });
  describe('needs you', () => {
    const waiting = (n: number, labels: string[] = [], author?: string) =>
      page(n, labels, {
        summary: { comments: 0, waiting: true, needsYou: 2, ...(author ? { author } : {}) },
      });
    const mixed = [
      waiting(1, ['design'], 'Studio agent'),
      page(2, ['design'], { summary: { comments: 1, waiting: false, needsYou: 0 } }),
      waiting(3, [], 'Review agent'),
      // A page whose summary is not known yet is never claimed to be waiting.
      page(4, ['design']),
    ];
    const ids = (needsYou: boolean | undefined, rest: Partial<PageFilter> = {}) =>
      filterPages(mixed, { labels: [], authors: [], text: '', needsYou, ...rest }, 'updated').map(
        (p) => p.id,
      );
    it('keeps only the pages with an item waiting on the viewer', () => {
      expect(ids(true)).toEqual([id(1), id(3)]);
      expect(ids(false)).toEqual([id(1), id(2), id(3), id(4)]);
      expect(ids(undefined)).toEqual([id(1), id(2), id(3), id(4)]);
    });
    it('combines with the label, creator and text filters', () => {
      expect(ids(true, { labels: ['design'] })).toEqual([id(1)]);
      expect(ids(true, { authors: ['Review agent'] })).toEqual([id(3)]);
      expect(ids(true, { text: 'studio' })).toEqual([id(1)]);
      expect(ids(true, { labels: ['missing'] })).toEqual([]);
    });
    it('counts the waiting pages it is given', () => {
      expect(waitingCount(mixed)).toBe(2);
      expect(waitingCount([])).toBe(0);
    });
  });
  describe('by creator', () => {
    const authored = [
      page(1, ['design'], {
        summary: { comments: 3, waiting: true, needsYou: 1, author: 'Studio agent' },
      }),
      page(2, [], {
        summary: { comments: 0, waiting: false, needsYou: 0, author: 'Review agent' },
      }),
      page(3, ['design'], { summary: { comments: 1, waiting: false, needsYou: 0 } }),
      page(4, [], {}),
    ];
    const ids = (authors: string[], labels: string[] = [], text = '') =>
      filterPages(authored, { labels, authors, text }, 'updated').map((p) => p.id);
    it('counts creators, leaving out pages whose creator is unknown', () => {
      expect(authorCounts(authored)).toEqual([
        { name: 'Review agent', count: 1 },
        { name: 'Studio agent', count: 1 },
      ]);
    });
    it('keeps the pages of any named creator, and none without one', () => {
      expect(ids(['Studio agent'])).toEqual([id(1)]);
      expect(ids(['Studio agent', 'Review agent'])).toEqual([id(1), id(2)]);
      expect(ids(['Nobody'])).toEqual([]);
      // A page with no known creator never matches a creator filter.
      expect(ids(['Studio agent']).includes(id(4))).toBe(false);
    });
    it('combines with a label and matches free text against the creator', () => {
      expect(ids(['Studio agent'], ['design'])).toEqual([id(1)]);
      expect(ids(['Review agent'], ['design'])).toEqual([]);
      expect(ids([], [], 'review agent')).toEqual([id(2)]);
      expect(ids([], [], 'studio q4')).toEqual([]);
    });
  });
  it('sorts by update time or title with the page ID as the tie-break', () => {
    const named = [
      page(1, [], { title: 'b' }),
      page(2, [], { title: 'a' }),
      page(3, [], { title: 'a' }),
    ];
    const order = (sort: 'updated' | 'title') =>
      filterPages(named, { labels: [], authors: [], text: '' }, sort).map((p) => p.id);
    expect(order('updated')).toEqual([id(1), id(2), id(3)]);
    expect(order('title')).toEqual([id(2), id(3), id(1)]);
  });
});

describe('label changes', () => {
  it('replace a page set whole, sorted, and refuse a ninth label', () => {
    expect(withLabel(page(1, ['b']), 'a', true)).toEqual({ pageId: id(1), labels: ['a', 'b'] });
    expect(withLabel(page(1, ['a', 'b']), 'a', false)).toEqual({ pageId: id(1), labels: ['b'] });
    expect(withLabel(page(1, ['a']), 'a', true)).toEqual({ pageId: id(1), labels: ['a'] });
    const full = Array.from({ length: 8 }, (_, n) => `l${n}`);
    expect(withLabel(page(1, full), 'new', true)).toBe('capacity');
    expect(withLabel(page(1, full), 'l3', true)).toEqual({ pageId: id(1), labels: full });
  });
  it('rename every page that carries the name, archived included, and merge onto an existing name', () => {
    const pages = [
      page(1, ['old', 'z']),
      page(2, ['new', 'old']),
      page(3, ['other']),
      page(4, ['old'], { archived: true }),
    ];
    expect(relabel(pages, 'old', 'new')).toEqual([
      { pageId: id(1), labels: ['new', 'z'] },
      { pageId: id(2), labels: ['new'] },
      { pageId: id(4), labels: ['new'] },
    ]);
    expect(relabel(pages, 'old')).toEqual([
      { pageId: id(1), labels: ['z'] },
      { pageId: id(2), labels: ['new'] },
      { pageId: id(4), labels: [] },
    ]);
    expect(relabel(pages, 'old', 'old')).toEqual([]);
    expect(relabel(pages, 'absent', 'x')).toEqual([]);
  });
  it('is idempotent: planning again after a partial write names only what is left', () => {
    const pages = Array.from({ length: 130 }, (_, n) => page(n + 1, ['old']));
    const plan = relabel(pages, 'old', 'new');
    const done = new Map(plan.slice(0, 64).map((c) => [c.pageId, c.labels]));
    const reread = pages.map((p) => (done.has(p.id) ? { ...p, projects: done.get(p.id)! } : p));
    expect(relabel(reread, 'old', 'new')).toHaveLength(66);
  });
});

describe('label requests', () => {
  const changes = (n: number): LabelChange[] =>
    Array.from({ length: n }, (_, k) => ({ pageId: id(n - k), labels: ['x'] }));
  it('are ascending, at most 64 pages, and the first page initiates', () => {
    const requests = labelRequests(changes(130));
    expect(requests.map((r) => r.length)).toEqual([64, 64, 2]);
    const ids = requests.flat().map((c) => c.pageId);
    expect(ids).toEqual([...ids].sort());
    expect(new Set(ids).size).toBe(130);
    expect(labelRequests(changes(64))).toHaveLength(1);
    expect(labelRequests([])).toEqual([]);
  });
  it('apply in order and stop at the first failed request', async () => {
    const sent: number[] = [];
    const writer = {
      async setLabels(request: readonly LabelChange[]) {
        if (sent.length === 1) throw new Error('boom');
        sent.push(request.length);
      },
    };
    const outcome = await applyLabelChanges(writer, changes(130));
    expect(sent).toEqual([64]);
    expect(outcome).toMatchObject({ updated: 64, total: 130 });
    expect(outcome.failure).toBeInstanceOf(Error);
    const all: number[] = [];
    const ok = await applyLabelChanges(
      { setLabels: async (request) => void all.push(request.length) },
      changes(130),
    );
    expect(all).toEqual([64, 64, 2]);
    expect(ok).toEqual({ updated: 130, total: 130 });
  });
});
