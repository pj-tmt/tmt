import { afterEach, expect, it, vi } from 'vite-plus/test';
import { deriveSpaceId, encodeBinary } from '@tmt/colab-client';
import { AUTHOR_BYTES, DISCOVERY_BYTES, PAGE_TITLE_BYTES, discover } from '../src/bootstrap.js';
import { record } from '../src/storage.js';
const storage = vi.hoisted(() => ({ pin: undefined as unknown }));
vi.mock('../src/storage.js', () => ({
  record: vi.fn(async (_key: string, ...values: unknown[]) =>
    values.length ? undefined : storage.pin,
  ),
}));
afterEach(() => {
  storage.pin = undefined;
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});
const mount = new URL('https://example.test/r/abcd/x/colab/');
const metadata = {
  pageId: '10000000-0000-4000-8000-000000000001',
  epoch: '18446744073709551615',
  sharing: 'private',
  history: 'current',
  archived: false,
  retentionDays: Number.MAX_SAFE_INTEGER,
  lastUpdateAtMs: Number.MAX_SAFE_INTEGER,
  expiresAtMs: null,
  warnings: ['not-in-cloud'],
  projects: [] as string[],
};
/** The reply's `projects` default to the names the pages carry; a project with no page needs `extra`. */
async function response(pages: unknown[], extra: unknown[] = []) {
  const owner = new Uint8Array(32).fill(9),
    space = await deriveSpaceId(owner);
  vi.stubGlobal('location', { hash: `#space=${space}` });
  const body = JSON.stringify({
    spaceId: space,
    ownerKey: encodeBinary(owner),
    revision: '1',
    projects: [
      ...new Set([
        ...pages.flatMap((page) => (page as { projects?: string[] }).projects ?? []),
        ...extra,
      ]),
    ].sort(),
    projectPins: { names: [], automatic: true },
    pages,
    pageIds: pages
      .map((page) => ({ pageId: (page as { pageId: string }).pageId, deleted: false }))
      .sort((a, b) => a.pageId.localeCompare(b.pageId)),
  });
  vi.stubGlobal('fetch', async () => new Response(body));
  return body;
}
it('preserves out-of-ID-order pins while matching the sorted catalog with deleted history', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const id = (n: number) => `10000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
  const pages = [
    { ...metadata, pageId: id(5), pinned: true, title: 'First pin' },
    { ...metadata, pageId: id(3), pinned: true, title: 'Second pin' },
    { ...metadata, pageId: id(1), title: 'Unpinned page' },
    { ...metadata, pageId: id(2), title: 'Another unpinned page' },
  ];
  const body = JSON.parse(await response(pages));
  body.pageIds.splice(3, 0, { pageId: id(4), deleted: true });
  vi.stubGlobal('fetch', async () => new Response(JSON.stringify(body)));
  const verify = vi.fn(async () => {});
  const boot = await discover(mount, verify);
  expect(boot.pages).toEqual(pages);
  expect(boot.pageIds).toEqual(body.pageIds);
  expect(verify).toHaveBeenCalledWith(boot.space, boot.owner);
});
it('a caller whose document is another module’s neither reads nor rewrites its location', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  await response([{ ...metadata, title: 'Alpha' }]);
  const reads: string[] = [];
  const replaceState = vi.fn();
  // The global search on the Ops page: a fragment that is not Colab's, on a path that is not Colab's.
  vi.stubGlobal('location', {
    origin: 'https://example.test',
    pathname: '/ops/',
    search: '',
    get hash() {
      reads.push('hash');
      return '#agents';
    },
  });
  vi.stubGlobal('history', { state: null, replaceState });
  // Colab's own discovery holds the document to Colab's grammar: this one it refuses.
  await expect(discover(mount)).rejects.toThrow();
  reads.length = 0;
  replaceState.mockClear();
  const boot = await discover(mount, undefined, undefined, { locate: false });
  expect(boot.pages).toHaveLength(1);
  expect(reads).toEqual([]);
  expect(replaceState).not.toHaveBeenCalled();
});
it('refuses duplicate or mismatched live IDs even when pins use display order', async () => {
  const id = (n: number) => `10000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
  const pages = [
    { ...metadata, pageId: id(3), pinned: true },
    { ...metadata, pageId: id(1) },
  ];
  const body = JSON.parse(await response(pages));
  const verify = vi.fn(async () => {});
  for (const patch of [
    { pages: [pages[0], pages[0], pages[1]] },
    { pages: [pages[0], { ...pages[1], pageId: id(2) }] },
    { pages: [pages[0]] },
    { pageIds: [...body.pageIds, { pageId: id(4), deleted: false }] },
    {
      pageIds: [
        { pageId: id(1), deleted: true },
        { pageId: id(3), deleted: false },
      ],
    },
    { pageIds: [...body.pageIds].reverse() },
    { pageIds: [body.pageIds[0], ...body.pageIds] },
  ]) {
    vi.stubGlobal('fetch', async () => new Response(JSON.stringify({ ...body, ...patch })));
    await expect(discover(mount, verify), JSON.stringify(patch)).rejects.toThrow();
  }
  expect(verify).not.toHaveBeenCalled();
  expect(record).not.toHaveBeenCalled();
});
it('accepts 1000 worst-sized valid expiry rows within the existing bounded response', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const pages = Array.from({ length: 1000 }, (_, index) => ({
    ...metadata,
    pageId: `10000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
  }));
  const body = await response(pages);
  expect(new TextEncoder().encode(body).length).toBeLessThan(DISCOVERY_BYTES);
  const boot = await discover(mount);
  expect(boot.pages).toHaveLength(1000);
  expect(boot.pages[999].lastUpdateAtMs).toBe(Number.MAX_SAFE_INTEGER);
  expect(boot.pages[0].warnings).toEqual(['not-in-cloud']);
});
it('keeps the worst-sized projects of a full space inside the bounded response', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  // The space limit is 1,000 project assignments: 125 pages of 8 names, each the longest allowed.
  const rockets = '\u{1F680}'.repeat(31);
  const pages = Array.from({ length: 1000 }, (_, index) => ({
    ...metadata,
    pageId: `10000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
    // 32 code points each (a digit and 31 four-byte characters), unique within the page.
    projects: index < 125 ? Array.from({ length: 8 }, (_, n) => `${n}${rockets}`) : [],
  }));
  const body = await response(pages);
  expect(new TextEncoder().encode(body).length).toBeLessThan(DISCOVERY_BYTES);
  const boot = await discover(mount);
  expect(boot.pages[0].projects).toHaveLength(8);
  expect(boot.pages[999].projects).toEqual([]);
  expect(boot.projects).toHaveLength(8);
});
it('keeps 1000 worst-sized summaries inside the bounded response', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  // Quotes escape to two bytes each in JSON, so 128 of them is the longest allowed author on the wire.
  const author = '"'.repeat(AUTHOR_BYTES);
  // The space lists every name once and each page names its own: 1,000 distinct names of 32 code
  // points (a space migrated from labels can hold that many), the largest reply the owner serves.
  const rockets = '\u{1F680}'.repeat(29);
  const pages = Array.from({ length: 1000 }, (_, index) => ({
    ...metadata,
    pageId: `10000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
    projects: [`${String(index).padStart(3, '0')}${rockets}`],
    summary: { comments: 4_294_967_295, waiting: true, needsYou: 350, author },
  }));
  const body = await response(pages);
  expect(new TextEncoder().encode(body).length).toBeLessThan(DISCOVERY_BYTES);
  const boot = await discover(mount);
  expect(boot.projects).toHaveLength(1000);
  expect(boot.pages[999].summary).toEqual({
    comments: 4_294_967_295,
    waiting: true,
    needsYou: 350,
    author,
  });
});
it('keeps 1000 worst-sized work rows and parents inside the bounded response', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  // The previous test's rows, each also carrying the longest work row and a parent. The three
  // assignee labels are 128 quotes, the longest allowed and two bytes each on the wire.
  const author = '"'.repeat(AUTHOR_BYTES);
  const rockets = '\u{1F680}'.repeat(29);
  const work = { stage: 'in-progress', done: 100, total: 256 };
  const pages = Array.from({ length: 1000 }, (_, index) => ({
    ...metadata,
    pageId: `10000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
    projects: [`${String(index).padStart(3, '0')}${rockets}`],
    state: 'in-progress',
    summary: {
      comments: 4_294_967_295,
      waiting: true,
      needsYou: 350,
      author,
      work,
      parent: { page: '10000000-0000-4000-8000-000000000001', index: 255 },
    },
  }));
  const body = await response(pages);
  const bytes = new TextEncoder().encode(body).length;
  expect(bytes).toBeLessThan(DISCOVERY_BYTES);
  // The measured worst case, so a field added later is budgeted against what is left.
  expect(bytes).toBeGreaterThan(1024 * 1024);
  const boot = await discover(mount);
  expect(boot.pages[999].summary?.work).toEqual(work);
  expect(boot.pages[999].summary?.parent).toEqual({
    page: '10000000-0000-4000-8000-000000000001',
    index: 255,
  });
});
it("accepts the owner's local page title and refuses one it never serves", async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const id = (n: number) => `10000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
  const longest = 'é'.repeat(PAGE_TITLE_BYTES / 2);
  await response([
    { ...metadata, pageId: id(1), title: 'Launch plan 🐈' },
    { ...metadata, pageId: id(2), title: longest },
    { ...metadata, pageId: id(3) },
  ]);
  const { pages } = await discover(mount);
  expect(pages.map((page) => page.title)).toEqual(['Launch plan 🐈', longest, undefined]);
  for (const title of ['', `${longest}x`, 'two\nlines', 'tab\there', 'nul\u0000', null, 7, ['x']]) {
    await response([{ ...metadata, pageId: id(1), title }]);
    await expect(discover(mount), JSON.stringify(title)).rejects.toThrow();
  }
});
it('keeps a reply the owner filled to its budget with titles inside the bounded response', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  // The owner spends what the worst-case reply leaves on titles, newest page first (limits.rs).
  const rockets = '\u{1F680}'.repeat(29);
  const author = '"'.repeat(AUTHOR_BYTES);
  const work = { stage: 'in-progress', done: 100, total: 256 };
  const page = (index: number, title?: string) => ({
    ...metadata,
    pageId: `10000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
    projects: [`${String(index).padStart(3, '0')}${rockets}`],
    summary: {
      comments: 4_294_967_295,
      waiting: true,
      needsYou: 350,
      author,
      work,
      parent: { page: '10000000-0000-4000-8000-000000000001', index: 255 },
    },
    ...(title === undefined ? {} : { title }),
  });
  const bare = new TextEncoder().encode(
    await response(Array.from({ length: 1000 }, (_, index) => page(index))),
  ).length;
  // Each longest title costs `,"title":"` + text + `"`; since rows carry no task detail, every
  // one of the 1,000 pages fits its longest title, and the owner's budget is never reached.
  const fit = Math.min(1000, Math.floor((DISCOVERY_BYTES - bare) / (10 + PAGE_TITLE_BYTES + 1)));
  expect(fit).toBe(1000);
  const body = await response(
    Array.from({ length: 1000 }, (_, index) =>
      page(index, index >= 1000 - fit ? 't'.repeat(PAGE_TITLE_BYTES) : undefined),
    ),
  );
  expect(new TextEncoder().encode(body).length).toBeLessThanOrEqual(DISCOVERY_BYTES);
  const boot = await discover(mount);
  expect(boot.pages.filter((p) => p.title !== undefined)).toHaveLength(fit);
});
it('accepts state-only list rows and refuses invalid state', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  await response([{ ...metadata, state: 'todo' }]);
  expect((await discover(mount)).pages[0].state).toBe('todo');
  for (const patch of [
    { state: 'unknown' },
    { state: 'todo', progress: { done: 0, inProgress: 0, todo: 0, unknown: 0, total: 0 } },
    { state: null, stateError: 'bad' },
  ]) {
    await response([{ ...metadata, ...patch }]);
    await expect(discover(mount), JSON.stringify(patch)).rejects.toThrow();
  }
  await response([{ ...metadata, state: null, stateError: 'page-unavailable' }]);
  expect((await discover(mount)).pages[0].stateError).toBe('page-unavailable');
});

it('accepts a work row and the page a page is a sub-page of, each on its own', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const id = (n: number) => `10000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
  const base = { comments: 0, waiting: false, needsYou: 0 };
  const rows = [
    { stage: 'discussion', done: 0, total: 0 },
    { stage: 'in-progress', done: 1, total: 3 },
    { stage: 'done', done: 3, total: 3 },
  ];
  await response([
    ...rows.map((work, n) => ({ ...metadata, pageId: id(n + 1), summary: { ...base, work } })),
    { ...metadata, pageId: id(9), summary: { ...base, parent: { page: id(1), index: 0 } } },
  ]);
  const { pages } = await discover(mount);
  expect(pages.slice(0, 3).map((page) => page.summary?.work)).toEqual(rows);
  expect(pages[3].summary).toEqual({ ...base, parent: { page: id(1), index: 0 } });
});
it('refuses a work row or a parent outside the bounds the owner derives', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const base = { comments: 0, waiting: false, needsYou: 0 };
  const row = { stage: 'in-progress', done: 1, total: 3 };
  const parent = { page: '10000000-0000-4000-8000-000000000001', index: 0 };
  for (const summary of [
    { ...base, work: null },
    { ...base, work: [] },
    { ...base, work: {} },
    { ...base, work: { ...row, extra: 1 } },
    { ...base, work: { ...row, stage: 'blocked' } },
    // The stage must follow the counts.
    { ...base, work: { ...row, stage: 'done' } },
    { ...base, work: { ...row, stage: 'discussion' } },
    { ...base, work: { ...row, done: 3, total: 3, stage: 'in-progress' } },
    { ...base, work: { ...row, done: 4, total: 3 } },
    { ...base, work: { ...row, total: 257 } },
    { ...base, work: { ...row, done: -1 } },
    { ...base, work: { ...row, done: 1.5 } },
    // The task-era fields are not part of a row.
    { ...base, work: { ...row, signoff: 0 } },
    { ...base, work: { ...row, working: [] } },
    { ...base, work: { stage: 'in-progress', done: 1 } },
    { ...base, parent: null },
    { ...base, parent: 'T-1' },
    { ...base, parent: {} },
    { ...base, parent: { page: parent.page } },
    { ...base, parent: { ...parent, extra: 1 } },
    { ...base, parent: { ...parent, page: 'not-a-page' } },
    { ...base, parent: { ...parent, page: 7 } },
    { ...base, parent: { ...parent, index: -1 } },
    { ...base, parent: { ...parent, index: 1.5 } },
    { ...base, parent: { ...parent, index: 256 } },
    { ...base, parent: { ...parent, index: '0' } },
    // The task-era parent link is not part of a row.
    { ...base, parent: { page: parent.page, task: 'T-1' } },
    { ...base, parent: { ...parent, title: 'Draft the export' } },
  ]) {
    await response([{ ...metadata, summary }]);
    await expect(discover(mount)).rejects.toThrow();
  }
});
it('accepts a summary with or without a creator and leaves absent summaries absent', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const id = (n: number) => `10000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
  await response([
    { ...metadata, pageId: id(1), summary: { comments: 0, waiting: false, needsYou: 0 } },
    {
      ...metadata,
      pageId: id(2),
      summary: { comments: 3, waiting: true, needsYou: 1, author: 'Ada \u{1F680}' },
    },
    { ...metadata, pageId: id(3) },
  ]);
  const { pages } = await discover(mount);
  expect(pages[0].summary).toEqual({ comments: 0, waiting: false, needsYou: 0 });
  expect(pages[1].summary).toEqual({
    comments: 3,
    waiting: true,
    needsYou: 1,
    author: 'Ada \u{1F680}',
  });
  expect('summary' in pages[2]).toBe(false);
});
it('refuses summaries outside the bounds the owner derives', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  for (const summary of [
    null,
    [],
    'x',
    {},
    { comments: 1 },
    { waiting: true, needsYou: 1 },
    { comments: -1, waiting: false, needsYou: 0 },
    { comments: 0, waiting: true, needsYou: 0 },
    { comments: 0, waiting: false, needsYou: 1 },
    { comments: 0, waiting: true, needsYou: 351 },
    { comments: 0, waiting: true, needsYou: 1.5 },
    { comments: 0, waiting: false, needsYou: -1 },
    { comments: 0, waiting: true, needsYou: '1' },
    { comments: 0, waiting: false },
    { comments: 1.5, waiting: false, needsYou: 0 },
    { comments: 4_294_967_296, waiting: false, needsYou: 0 },
    { comments: '1', waiting: false, needsYou: 0 },
    { comments: 1, waiting: 'yes' },
    { comments: 1, waiting: false, needsYou: 0, extra: 1 },
    { comments: 1, waiting: false, needsYou: 0, author: '' },
    { comments: 1, waiting: false, needsYou: 0, author: 7 },
    { comments: 1, waiting: false, needsYou: 0, author: 'a'.repeat(AUTHOR_BYTES + 1) },
    { comments: 1, waiting: false, needsYou: 0, author: '\u{1F680}'.repeat(33) },
    { comments: 1, waiting: false, needsYou: 0, author: 'tab\there' },
  ]) {
    await response([{ ...metadata, summary }]);
    await expect(discover(mount)).rejects.toThrow();
  }
  // The longest allowed author is accepted at the byte boundary.
  await response([
    {
      ...metadata,
      summary: { comments: 1, waiting: false, needsYou: 0, author: 'a'.repeat(AUTHOR_BYTES) },
    },
  ]);
  expect((await discover(mount)).pages[0].summary?.author).toHaveLength(AUTHOR_BYTES);
});
it('accepts sorted display-only projects and refuses what the owner store never writes', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  await response([{ ...metadata, projects: ['a b', 'q4 plan', '日本語'] }], ['empty']);
  const boot = await discover(mount);
  expect(boot.pages[0].projects).toEqual(['a b', 'q4 plan', '日本語']);
  // A project with no page is listed for the space and in no entry.
  expect(boot.projects).toEqual(['a b', 'empty', 'q4 plan', '日本語']);
  for (const projects of [
    'urgent',
    [1],
    [''],
    ['a', 'a'],
    ['x'.repeat(33)],
    ['tab\there'],
    Array.from({ length: 9 }, (_, n) => `l${n}`),
    null,
  ]) {
    await response([{ ...metadata, projects }]);
    await expect(discover(mount)).rejects.toThrow();
  }
  // A page without the key is not the shape the owner serves.
  const { projects: _omitted, ...bare } = metadata;
  await response([bare]);
  await expect(discover(mount)).rejects.toThrow();
});
it('refuses a malformed space project list or an entry naming a project the space lacks', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const owner = new Uint8Array(32).fill(9),
    space = await deriveSpaceId(owner);
  vi.stubGlobal('location', { hash: `#space=${space}` });
  const reply = (projects: unknown, pages: unknown[] = []) => {
    const body = JSON.stringify({
      spaceId: space,
      ownerKey: encodeBinary(owner),
      revision: '1',
      ...(projects === undefined ? {} : { projects }),
      projectPins: { names: [], automatic: true },
      pages,
      pageIds: pages.map((page) => ({
        pageId: (page as { pageId: string }).pageId,
        deleted: false,
      })),
    });
    vi.stubGlobal('fetch', async () => new Response(body));
  };
  reply([]);
  await expect(discover(mount)).resolves.toMatchObject({ projects: [] });
  for (const projects of [
    undefined,
    null,
    'a',
    [1],
    [''],
    ['a', 'a'],
    ['x'.repeat(33)],
    ['tab\there'],
    Array.from({ length: 1001 }, (_, n) => `p${n}`),
  ]) {
    reply(projects);
    await expect(discover(mount)).rejects.toThrow();
  }
  // The page names a project the space does not list.
  reply(['a'], [{ ...metadata, projects: ['b'] }]);
  await expect(discover(mount)).rejects.toThrow();
});
it('accepts the owner pinned projects only as an ordered, unique list of projects the space has', async () => {
  const owner = new Uint8Array(32).fill(9),
    space = await deriveSpaceId(owner);
  vi.stubGlobal('location', { hash: `#space=${space}` });
  const reply = (pins: unknown, projects: string[] = ['a', 'b', 'c']) => {
    const body = JSON.stringify({
      spaceId: space,
      ownerKey: encodeBinary(owner),
      revision: '1',
      projects,
      ...(pins === undefined ? {} : { projectPins: pins }),
      pages: [],
      pageIds: [],
    });
    vi.stubGlobal('fetch', async () => new Response(body));
  };
  reply({ names: ['c', 'a'], automatic: false });
  await expect(discover(mount)).resolves.toMatchObject({
    projectPins: { names: ['c', 'a'], automatic: false },
  });
  // The owner's empty list is a choice and arrives as one.
  reply({ names: [], automatic: false });
  await expect(discover(mount)).resolves.toMatchObject({ projectPins: { names: [] } });
  const twenty = Array.from({ length: 20 }, (_, n) => `p${n}`);
  reply({ names: twenty, automatic: false }, twenty);
  await expect(discover(mount)).resolves.toMatchObject({ projectPins: { names: twenty } });
  for (const pins of [
    undefined,
    null,
    [],
    { names: ['a'] },
    { automatic: true },
    { names: ['a'], automatic: 'yes' },
    { names: ['a'], automatic: true, extra: 1 },
    { names: 'a', automatic: true },
    { names: ['a', 'a'], automatic: true },
    { names: ['missing'], automatic: true },
    { names: [1], automatic: true },
    { names: [''], automatic: true },
    { names: ['tab\there'], automatic: true },
  ]) {
    reply(pins);
    await expect(discover(mount)).rejects.toThrow();
  }
  // A 21st pin is past what the owner stores.
  const over = Array.from({ length: 21 }, (_, n) => `p${n}`);
  reply({ names: over, automatic: false }, over);
  await expect(discover(mount)).rejects.toThrow();
});
it('rejects unsafe or ill-typed expiry hints before accepting discovery', async () => {
  for (const patch of [
    { lastUpdateAtMs: -1 },
    { lastUpdateAtMs: 1.5 },
    { expiresAtMs: Number.MAX_SAFE_INTEGER + 1 },
    { retentionDays: 0 },
    { retentionDays: '30' },
    { warnings: ['unknown'] },
    { warnings: ['not-in-cloud', 'expires-soon'] },
    { warnings: ['expired'] },
    { warnings: ['expiry-unavailable'] },
    { warnings: [1] },
  ]) {
    await response([{ ...metadata, ...patch }]);
    await expect(discover(mount)).rejects.toThrow();
  }
});
it('rejects oversized discovery bodies even without Content-Length', async () => {
  await response([]);
  vi.stubGlobal('fetch', async () => new Response(' '.repeat(DISCOVERY_BYTES + 1)));
  await expect(discover(mount)).rejects.toThrow('exceeds limit');
});

it('retains an anonymous short-link target across authenticated discovery and mount pinning', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  await response([]);
  vi.stubGlobal('location', { hash: '#path=%2Fshort%2F12345678' });
  const replaceState = vi.fn();
  vi.stubGlobal('history', { state: null, replaceState });
  const boot = await discover(mount);
  expect(replaceState).toHaveBeenCalledWith(
    null,
    '',
    `${mount.pathname}#space=${boot.space}&path=%2Fshort%2F12345678`,
  );
});
it('retains a native page deep link target and refuses a malformed page ID', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  await response([]);
  const page = '10000000-0000-4000-8000-000000000001';
  vi.stubGlobal('location', { hash: `#path=%2Fpages%2F${page}` });
  const replaceState = vi.fn();
  vi.stubGlobal('history', { state: null, replaceState });
  const boot = await discover(mount);
  expect(replaceState).toHaveBeenCalledWith(
    null,
    '',
    `${mount.pathname}#space=${boot.space}&path=%2Fpages%2F${page}`,
  );
  await response([]);
  vi.stubGlobal('location', { hash: '#path=%2Fpages%2F10000000' });
  await expect(discover(mount)).rejects.toThrow();
});
it('rejects secret or malformed fragments instead of retaining them in a short-link recovery', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  for (const hash of [
    '#seed=secret',
    '#path=%2Fshort%2F1234567',
    '#path=%2Fshort%2F12345678&path=%2Fpages%2Ffoo',
  ]) {
    await response([]);
    vi.stubGlobal('location', { hash });
    await expect(discover(mount)).rejects.toThrow();
  }
});

it('rejects inconsistent or title-bearing tombstone metadata before pinning', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  for (const pageIds of [
    [{ pageId: metadata.pageId, deleted: true }],
    [{ pageId: metadata.pageId, deleted: false, title: 'deleted text' }],
    [{ pageId: metadata.pageId, deleted: 'true' }],
    [],
  ]) {
    const body = JSON.parse(await response([metadata]));
    vi.stubGlobal('fetch', async () => new Response(JSON.stringify({ ...body, pageIds })));
    await expect(discover(mount)).rejects.toThrow();
  }
});

it('public page and thread targets do not replace the internal mount or bypass owner verification', async () => {
  await response([metadata]);
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  vi.stubGlobal('location', { pathname: '/p/10000000', hash: `#t=${metadata.pageId}` });
  const replaceState = vi.fn();
  vi.stubGlobal('history', { state: null, replaceState });
  const verify = vi.fn(async () => {});
  const boot = await discover(mount, verify);
  expect(verify).toHaveBeenCalledWith(boot.space, boot.owner);
  expect(record).toHaveBeenCalledWith(`pin:${mount.href}`, {
    space: boot.space,
    owner: boot.owner,
  });
  expect(replaceState).not.toHaveBeenCalled();
});
it('public entry refuses unexpected fragments and a mismatched existing pin without re-pinning', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const replaceState = vi.fn();
  vi.stubGlobal('history', { state: null, replaceState });
  for (const hash of [
    '#seed=secret',
    '#space=wrong',
    '#path=%2Fshort%2F12345678',
    `#t=${metadata.pageId}&path=other`,
  ]) {
    await response([metadata]);
    vi.stubGlobal('location', { pathname: '/p/10000000', hash });
    await expect(discover(mount)).rejects.toThrow();
  }
  storage.pin = { space: 'b'.repeat(32), owner: new Uint8Array(32).fill(9) };
  await response([metadata]);
  vi.stubGlobal('location', { pathname: '/p/10000000', hash: '' });
  await expect(discover(mount)).rejects.toThrow();
  expect(vi.mocked(record).mock.calls.every((call) => Array.from(call).length === 1)).toBe(true);
  expect(replaceState).not.toHaveBeenCalled();
});
it('public entry opens on a plain #id or a fragment naming no display target, and keeps only the canonical fragment', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const replaceState = vi.fn();
  vi.stubGlobal('history', { state: null, replaceState });
  for (const hash of ['#anchor=digest-matrix', `#t=${metadata.pageId}`, '']) {
    await response([metadata]);
    vi.stubGlobal('location', { pathname: '/p/10000000', search: '', hash });
    await expect(discover(mount)).resolves.toBeDefined();
  }
  expect(replaceState).not.toHaveBeenCalled();
  for (const [hash, kept] of [
    ['#digest-matrix', '#anchor=digest-matrix'],
    ['#%%%garbage', ''],
    ['#a=1&b=2', ''],
    ['#t=bad', ''],
    ['#read=bad', ''],
    [`#t=${metadata.pageId}&t=${metadata.pageId}`, ''],
  ]) {
    replaceState.mockClear();
    await response([metadata]);
    vi.stubGlobal('location', { pathname: '/p/10000000', search: '?x=1', hash });
    await expect(discover(mount)).resolves.toBeDefined();
    expect(replaceState).toHaveBeenCalledExactlyOnceWith(null, '', `/p/10000000?x=1${kept}`);
  }
});
it('an authenticated legacy page entry canonicalizes to a catalog-derived short target', async () => {
  await response([metadata]);
  const space = await deriveSpaceId(new Uint8Array(32).fill(9));
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const hash = `#space=${space}&path=%2Fpages%2F${metadata.pageId}`;
  vi.stubGlobal('location', { href: `${mount.href}${hash}`, pathname: mount.pathname, hash });
  const replaceState = vi.fn();
  vi.stubGlobal('history', { state: null, replaceState });
  await discover(mount);
  expect(replaceState).toHaveBeenCalledWith(null, '', '/p/10000000');
});
it('an authenticated legacy page entry keeps the owner reading hint on the short target', async () => {
  await response([metadata]);
  const space = await deriveSpaceId(new Uint8Array(32).fill(9));
  const read = '10000000-0000-4000-8000-000000000401';
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  vi.stubGlobal('location', {
    href: `${mount.href}#space=${space}&path=%2Fpages%2F${metadata.pageId}&read=${read}`,
    pathname: mount.pathname,
    hash: `#space=${space}&path=%2Fpages%2F${metadata.pageId}&read=${read}`,
  });
  const replaceState = vi.fn();
  vi.stubGlobal('history', { state: null, replaceState });
  await discover(mount);
  expect(replaceState).toHaveBeenCalledWith(null, '', `/p/10000000#read=${read}`);
});
