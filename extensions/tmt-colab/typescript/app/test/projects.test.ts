import { pinMenuItem } from '../src/pins.js';
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import {
  applyProjectChanges,
  compareProjects,
  filterPages,
  authorCounts,
  projectCounts,
  projectNameError,
  projectRequests,
  withProject,
  waitingCount,
  type ProjectChange,
  type PageFilter,
} from '../src/projects.js';
import { pageStage, stageCounts } from '../src/list-stage.js';
import type { PageSummary } from '../src/transport.js';

const id = (n: number) => `10000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
function page(n: number, projects: string[], changes: Partial<PageSummary> = {}): PageSummary {
  return {
    id: id(n),
    title: `Page ${n}`,
    sharing: 'private',
    lastUpdateAtMs: 1000 - n,
    projects,
    ...changes,
  };
}

describe('project names', () => {
  it('follow the owner store grammar', () => {
    for (const name of ['a', 'Q4 plan', '日本語', '🚀 launch', 'x'.repeat(32)])
      expect(projectNameError(name), name).toBeUndefined();
    expect(projectNameError('')).toBe('empty');
    expect(projectNameError('x'.repeat(33))).toBe('long');
    // 32 astral characters are 32 points, not 64 UTF-16 units.
    expect(projectNameError('🚀'.repeat(32))).toBeUndefined();
    for (const name of [' a', 'a ', 'a  b', 'a\tb', 'a\nb', 'a b', 'a\u0000b', '　'])
      expect(projectNameError(name), JSON.stringify(name)).toBe('form');
  });
  // The owner store reads the same file (projects.rs): the two grammars cannot drift. U+FEFF and
  // U+200B are not White_Space, so a CLI project carrying one must stay editable here.
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
      expect(projectNameError(name), JSON.stringify(name)).toBeUndefined();
    for (const name of vector.invalid)
      expect(projectNameError(name), JSON.stringify(name)).toBeDefined();
    expect(projectNameError('a\ufeffb')).toBeUndefined();
  });
  it('sort by code point like the owner store, not by UTF-16 unit', () => {
    // U+FF5E sorts before U+1F680 by code point, after it by UTF-16 unit.
    expect(compareProjects('～', '🚀')).toBeLessThan(0);
    expect(['b', 'a', 'B', '日'].sort(compareProjects)).toEqual(['B', 'a', 'b', '日']);
    expect(compareProjects('a', 'ab')).toBeLessThan(0);
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
    expect(projectCounts(pages)).toEqual([
      { name: 'design', count: 2 },
      { name: 'ops', count: 1 },
      { name: 'q4 plan', count: 1 },
    ]);
    expect(projectCounts(pages.filter((p) => !p.archived))).toHaveLength(2);
  });
  it('requires every label token and every word in the title or a label', () => {
    const ids = (projects: string[], text: string) =>
      filterPages(pages, { projects, authors: [], text }, 'updated').map((p) => p.id);
    expect(ids([], '')).toEqual([id(1), id(2), id(3), id(4)]);
    expect(ids(['design'], '')).toEqual([id(1), id(2)]);
    expect(ids(['design', 'q4 plan'], '')).toEqual([id(1)]);
    expect(ids([], 'DESIGN')).toEqual([id(1), id(2), id(3)]);
    expect(ids([], 'design notes')).toEqual([id(3)]);
    expect(ids([], 'q4 page')).toEqual([id(1)]);
    expect(ids(['missing'], '')).toEqual([]);
  });
  it('leads with the pinned pages in the order given, under either sort and any filter', () => {
    const pinned = [
      { ...page(3, [], { title: 'Zulu' }), pinned: true },
      { ...page(2, ['design']), pinned: true },
    ];
    const list = [pages[0], ...pinned, page(5, ['ops'])];
    const ids = (sort: 'updated' | 'title', projects: string[] = []) =>
      filterPages(list, { projects, authors: [], text: '' }, sort).map((p) => p.id);
    expect(ids('updated')).toEqual([id(3), id(2), id(1), id(5)]);
    expect(ids('title')).toEqual([id(3), id(2), id(1), id(5)]);
    // A filter still applies to a pinned page; the ones that remain keep their pin order.
    expect(ids('updated', ['design'])).toEqual([id(2), id(1)]);
    // An archived page keeps no pin in the list.
    expect(
      filterPages(
        [{ ...page(4, [], { archived: true }), pinned: true }, pages[0]],
        {
          projects: [],
          authors: [],
          text: '',
        },
        'updated',
      ).map((p) => p.id),
    ).toEqual([id(1), id(4)]);
  });
  describe('stages', () => {
    const activity = { comments: 0, waiting: false, needsYou: 0 };
    const row = (stage: 'discussion' | 'in-progress' | 'done', done: number, total: number) => ({
      stage,
      done,
      total,
      signoff: 0,
      working: [],
    });
    const staged = [
      page(1, [], { summary: { ...activity, work: row('in-progress', 1, 3) } }),
      page(2, [], { summary: { ...activity, work: row('done', 2, 2) } }),
      // A fresh summary without work is a page with no task.
      page(3, [], { summary: activity }),
      // A page whose summary is not known yet is at no stage.
      page(4, []),
      page(5, [], { archived: true, summary: { ...activity, work: row('in-progress', 0, 1) } }),
    ];
    it('reads the stage from the summary, never inventing one for a page that has none', () => {
      expect(staged.map(pageStage)).toEqual([
        'in-progress',
        'done',
        'discussion',
        undefined,
        'in-progress',
      ]);
    });
    it('counts the pages it is given at each stage and leaves the unknown ones out', () => {
      expect(stageCounts(staged.filter((p) => !p.archived))).toEqual({
        'in-progress': 1,
        discussion: 1,
        done: 1,
      });
      expect(stageCounts([])).toEqual({ 'in-progress': 0, discussion: 0, done: 0 });
    });
    it('keeps only the pages at the stage, alone or with the other filters', () => {
      const ids = (stage: 'in-progress' | 'discussion' | 'done' | undefined, text = '') =>
        filterPages(staged, { projects: [], authors: [], text, stage }, 'updated').map((p) => p.id);
      expect(ids('in-progress')).toEqual([id(1), id(5)]);
      expect(ids('done')).toEqual([id(2)]);
      expect(ids('discussion')).toEqual([id(3)]);
      expect(ids(undefined)).toHaveLength(5);
      expect(ids('in-progress', 'page 5')).toEqual([id(5)]);
    });
  });
  describe('needs you', () => {
    const waiting = (n: number, projects: string[] = [], author?: string) =>
      page(n, projects, {
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
      filterPages(mixed, { projects: [], authors: [], text: '', needsYou, ...rest }, 'updated').map(
        (p) => p.id,
      );
    it('keeps only the pages with an item waiting on the viewer', () => {
      expect(ids(true)).toEqual([id(1), id(3)]);
      expect(ids(false)).toEqual([id(1), id(2), id(3), id(4)]);
      expect(ids(undefined)).toEqual([id(1), id(2), id(3), id(4)]);
    });
    it('combines with the label, creator and text filters', () => {
      expect(ids(true, { projects: ['design'] })).toEqual([id(1)]);
      expect(ids(true, { authors: ['Review agent'] })).toEqual([id(3)]);
      expect(ids(true, { text: 'studio' })).toEqual([id(1)]);
      expect(ids(true, { projects: ['missing'] })).toEqual([]);
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
    const ids = (authors: string[], projects: string[] = [], text = '') =>
      filterPages(authored, { projects, authors, text }, 'updated').map((p) => p.id);
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
      filterPages(named, { projects: [], authors: [], text: '' }, sort).map((p) => p.id);
    expect(order('updated')).toEqual([id(1), id(2), id(3)]);
    expect(order('title')).toEqual([id(2), id(3), id(1)]);
  });
});

describe('project changes', () => {
  it('replace a page set whole, sorted, and refuse a ninth project', () => {
    expect(withProject(page(1, ['b']), 'a', true)).toEqual({ pageId: id(1), projects: ['a', 'b'] });
    expect(withProject(page(1, ['a', 'b']), 'a', false)).toEqual({
      pageId: id(1),
      projects: ['b'],
    });
    expect(withProject(page(1, ['a']), 'a', true)).toEqual({ pageId: id(1), projects: ['a'] });
    const full = Array.from({ length: 8 }, (_, n) => `l${n}`);
    expect(withProject(page(1, full), 'new', true)).toBe('capacity');
    expect(withProject(page(1, full), 'l3', true)).toEqual({ pageId: id(1), projects: full });
  });
  it('count the pages in each project and list the space projects that no page is in', () => {
    const pages = [page(1, ['b', 'a']), page(2, ['a'])];
    expect(projectCounts(pages)).toEqual([
      { name: 'a', count: 2 },
      { name: 'b', count: 1 },
    ]);
    expect(projectCounts(pages, ['a', 'b', 'empty'])).toEqual([
      { name: 'a', count: 2 },
      { name: 'b', count: 1 },
      { name: 'empty', count: 0 },
    ]);
    expect(projectCounts([], ['z', 'a'])).toEqual([
      { name: 'a', count: 0 },
      { name: 'z', count: 0 },
    ]);
  });
});

describe('project requests', () => {
  const changes = (n: number): ProjectChange[] =>
    Array.from({ length: n }, (_, k) => ({ pageId: id(n - k), projects: ['x'] }));
  it('are ascending, at most 64 pages, and the first page initiates', () => {
    const requests = projectRequests(changes(130));
    expect(requests.map((r) => r.length)).toEqual([64, 64, 2]);
    const ids = requests.flat().map((c) => c.pageId);
    expect(ids).toEqual([...ids].sort());
    expect(new Set(ids).size).toBe(130);
    expect(projectRequests(changes(64))).toHaveLength(1);
    expect(projectRequests([])).toEqual([]);
  });
  it('apply in order and stop at the first failed request', async () => {
    const sent: number[] = [];
    const writer = {
      async setProjects(request: readonly ProjectChange[]) {
        if (sent.length === 1) throw new Error('boom');
        sent.push(request.length);
      },
      manageProject: async () => {},
    };
    const outcome = await applyProjectChanges(writer, changes(130));
    expect(sent).toEqual([64]);
    expect(outcome).toMatchObject({ updated: 64, total: 130 });
    expect(outcome.failure).toBeInstanceOf(Error);
    const all: number[] = [];
    const ok = await applyProjectChanges(
      {
        setProjects: async (request) => void all.push(request.length),
        manageProject: async () => {},
      },
      changes(130),
    );
    expect(all).toEqual([64, 64, 2]);
    expect(ok).toEqual({ updated: 130, total: 130 });
  });
});

describe('the phone page-header pin item', () => {
  const state = { pinned: false, busy: false };
  it('follows the admitted pin state and disables a pending write', () => {
    expect(pinMenuItem(state, true, true, false)).toEqual({
      key: 'pin',
      label: 'Pin to top',
      disabled: false,
    });
    expect(pinMenuItem({ pinned: true, busy: false }, true, true, false)).toEqual({
      key: 'pin',
      label: 'Unpin',
      disabled: false,
    });
    expect(pinMenuItem({ ...state, busy: true }, true, true, false)?.disabled).toBe(true);
  });
  it('offers nothing at desktop width, before the owner list, archived, or without write access', () => {
    expect(pinMenuItem(state, false, true, false)).toBeUndefined();
    expect(pinMenuItem(undefined, true, true, false)).toBeUndefined();
    expect(pinMenuItem(state, true, false, false)).toBeUndefined();
    expect(pinMenuItem(state, true, true, true)).toBeUndefined();
  });
});
