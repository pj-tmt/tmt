/** Shell fixtures use the real router and isolated Worker; no production transport changes. */
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import * as Y from 'yjs';
import { createAppRouter } from '../src/router.js';
import { Fold } from '../src/fold.js';
import { ReaderApp } from '../src/reader-app.js';
import type { ReadingDetails } from '../src/reading-details.js';
import type { Shell } from '../src/shell.js';
import { startOfDay } from '../src/shell-model.js';
import type { PageSummary } from '../src/transport.js';

const SHELL = '10000000-0000-4000-8000-000000000001';
const PLAN = '10000000-0000-4000-8000-0000000000a1';
const NOTES = '10000000-0000-4000-8000-0000000000a2';
const OLD = '10000000-0000-4000-8000-0000000000a3';
export const DRAFT = '10000000-0000-4000-8000-0000000000a4';
const EARLIER = '10000000-0000-4000-8000-0000000000a5';
const ARCHIVED = '10000000-0000-4000-8000-0000000000a6';
const GONE = '10000000-0000-4000-8000-0000000000ff';
const item = (n: number) => `20000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
export const READING = '30000000-0000-4000-8000-0000000000b1';
const REVIEW = '30000000-0000-4000-8000-0000000000b2';
const QUESTIONS = '30000000-0000-4000-8000-0000000000b3';

/** A shell with a plan, notes, a weekly report, older pages, a vanished page and an archived one. */
function fullShell(summaryAt: number): Shell {
  return {
    version: 1,
    items: {
      [item(1)]: {
        kind: 'page',
        pageId: PLAN,
        note: 'Start with the plan.',
        audience: 'Reviewers',
      },
      [item(2)]: { kind: 'anchor', pageId: NOTES, anchor: 'decisions' },
      [item(3)]: { kind: 'page', pageId: GONE, note: 'This page was deleted.' },
      [item(4)]: { kind: 'page', pageId: OLD },
      [item(5)]: { kind: 'page', pageId: DRAFT },
      [item(6)]: { kind: 'page', pageId: EARLIER },
      [item(7)]: { kind: 'page', pageId: ARCHIVED },
      [item(8)]: {
        kind: 'page',
        pageId: PLAN,
        note: `A long note that has to wrap. ${'word '.repeat(60)}`,
      },
    },
    order: [item(1), item(2), item(3), item(4), item(5), item(6), item(7), item(8)],
    summary: {
      line1: 'Release is on track',
      line2: 'Two reviews still need a decision before Friday.',
      attribution: {
        writerId: '40000000-0000-4000-8000-000000000001',
        timeMs: summaryAt,
        displayAgent: { name: 'Atlas' },
      },
    },
  };
}

/** The same pages with a reading item that names the pending reading, so it leads its chapter. */
function namedShell(summaryAt: number): Shell {
  const full = fullShell(summaryAt);
  return {
    ...full,
    items: {
      ...full.items,
      [item(9)]: { kind: 'reading', pageId: PLAN, readingId: READING },
    },
    order: [...full.order, item(9)],
  };
}

export type ShellCase =
  | 'full'
  | 'empty'
  | 'no-summary'
  | 'pages-failed'
  | 'not-admitted'
  | 'settled'
  | 'named'
  | 'details-failed'
  | 'details-many';

const manyPage = (n: number) =>
  `10000000-0000-4000-8000-${(0x1000 + n).toString(16).padStart(12, '0')}`;

declare global {
  interface Window {
    /** Page IDs asked for chapter details, in order. */
    detailCalls: string[];
  }
}

export function mount(kind: ShellCase = 'full') {
  const now = Date.now();
  // Today's pages stay today however early the run starts, so the counts never depend on the hour.
  const earliestToday = startOfDay(now) + 1;
  const ago = (ms: number) => Math.max(now - ms, earliestToday);
  const rows: PageSummary[] = [
    { id: SHELL, title: 'Release shell', sharing: 'private' as const },
    {
      id: PLAN,
      title: 'Release plan',
      sharing: 'private' as const,
      lastUpdateAtMs: ago(2 * MINUTE),
      projects: ['Design'],
      summary: { comments: 3, waiting: true, needsYou: 2, author: 'tmt-feat-lead' },
    },
    {
      id: NOTES,
      title: 'Design notes',
      sharing: 'private' as const,
      lastUpdateAtMs: ago(HOUR),
      projects: ['Remote'],
      summary: { comments: 1, waiting: true, needsYou: 1, author: 'tmt-remote-lead' },
    },
    {
      id: DRAFT,
      title: 'Weekly report',
      sharing: 'private' as const,
      lastUpdateAtMs: ago(3 * HOUR),
      projects: ['Report'],
      summary: { comments: 2, waiting: false, needsYou: 0, author: 'tmt-core-pm' },
    },
    {
      id: OLD,
      title: 'Last quarter',
      sharing: 'private' as const,
      lastUpdateAtMs: now - 20 * DAY,
      summary: { comments: 0, waiting: false, needsYou: 0, author: 'tmt-ux-lead' },
    },
    {
      id: EARLIER,
      title: 'Usage chart round two',
      sharing: 'private' as const,
      lastUpdateAtMs: now - 9 * DAY,
    },
    {
      id: ARCHIVED,
      title: 'Archived plan',
      sharing: 'private' as const,
      archived: true,
      lastUpdateAtMs: now - DAY,
    },
  ];
  const full = fullShell(ago(3 * HOUR));
  const shell: Shell =
    kind === 'empty'
      ? { version: 1, items: {}, order: [] }
      : kind === 'no-summary'
        ? { ...full, summary: undefined }
        : kind === 'settled'
          ? { ...full, items: { [item(5)]: full.items[item(5)] }, order: [item(5)] }
          : kind === 'named'
            ? namedShell(ago(3 * HOUR))
            : kind === 'details-many'
              ? manyShell()
              : full;
  if (kind === 'details-many')
    for (let n = 0; n < 25; n++)
      rows.push({
        id: manyPage(n),
        title: `Page ${n}`,
        sharing: 'private' as const,
        lastUpdateAtMs: now - n * MINUTE,
        projects: [],
        summary: { comments: 0, waiting: true, needsYou: 1, author: 'tmt-feat-lead' },
      });
  window.detailCalls = [];
  const ready = (
    id: string,
    needs: NonNullable<Extract<ReadingDetails, { state: 'ready' }>['needs']>,
    latest?: { atMs: number; text: string; who?: string },
  ): ReadingDetails => ({
    pageId: id,
    state: 'ready',
    fingerprint: 'a'.repeat(64),
    summary: { comments: 0, waiting: needs.length > 0, needsYou: needs.length },
    readings: [],
    needs,
    ...(latest ? { latest } : {}),
  });
  const readingDetails =
    kind === 'full' || kind === 'named' || kind === 'details-failed' || kind === 'details-many'
      ? async (id: string): Promise<ReadingDetails> => {
          window.detailCalls.push(id);
          if (kind === 'details-failed') throw new Error('offline');
          if (id === PLAN)
            return ready(id, [
              {
                kind: 'review',
                id: REVIEW,
                title: 'Review the release scope',
                agent: 'tmt-feat-lead',
              },
              {
                kind: 'reading',
                id: READING,
                title: 'Decide the scope',
                agent: 'Atlas',
                why: 'Read before the review',
              },
            ]);
          if (id === NOTES)
            return ready(id, [
              {
                kind: 'question',
                id: QUESTIONS,
                title: 'Which hosting plan?',
                agent: 'tmt-remote-lead',
              },
            ]);
          if (id === DRAFT)
            return ready(id, [], {
              atMs: now - 3 * HOUR,
              text: 'Merged the review blocks.',
              who: 'tmt-core-pm',
            });
          if (id.startsWith('10000000-0000-4000-8000-0000000010'))
            return ready(id, [
              {
                kind: 'proposal',
                id: `${id.slice(0, -4)}f0f0`,
                title: `Proposal on ${id.slice(-4)}`,
                agent: 'Atlas',
              },
            ]);
          return { pageId: id, state: 'pending' };
        }
      : undefined;
  location.hash = `/pages/${SHELL}`;
  const router = createAppRouter({
    backendName: 'Fixture device',
    ...(readingDetails ? { readingDetails } : {}),
    async spaceHome() {
      if (kind === 'pages-failed' && location.hash.endsWith(SHELL)) throw new Error('offline');
      return { title: 'Workspace', pages: rows };
    },
    async page(id: string) {
      const row = rows.find((candidate) => candidate.id === id) ?? rows[0];
      return id === SHELL
        ? { ...row, source: '', pageType: 'shell', ...(kind === 'not-admitted' ? {} : { shell }) }
        : {
            ...row,
            source: `<h1>${row.title}</h1><div style="height:2400px">Spacer</div><section id="decisions"><h2>Decisions</h2></section><div style="height:2400px">End</div>`,
          };
    },
  });
  createRoot(document.getElementById('root')!).render(<RouterProvider router={router} />);
}

/** Twenty-five waiting pages, to see that only the first twenty chapters are asked about. */
function manyShell(): Shell {
  const ids = Array.from({ length: 25 }, (_, n) => item(100 + n));
  return {
    version: 1,
    items: Object.fromEntries(
      ids.map((id, n) => [id, { kind: 'page' as const, pageId: manyPage(n) }]),
    ),
    order: ids,
  };
}
export function mountReader() {
  createRoot(document.getElementById('root')!).render(
    <ReaderApp
      state={{
        kind: 'ready',
        pageId: '10000000-0000-4000-8000-000000000001',
        view: {
          source: '',
          title: 'Shared shell',
          pageType: 'shell',
          shell: { version: 1, items: {}, order: [] },
        },
      }}
      canEdit
      edit={async () => {
        throw new Error('A shell has no source edit');
      }}
    />,
  );
}
export async function checkCases(cases: { name: string; document: unknown; action?: string }[]) {
  const results = [];
  for (const fixture of cases) {
    if (fixture.action) continue;
    const doc = new Y.Doc();
    doc.getText('html');
    const meta = doc.getMap('meta');
    doc.transact(() => {
      meta.set('title', 'Shell');
      meta.set('pageType', 'shell');
      meta.set('shell', fixture.document);
    });
    const fold = new Fold();
    try {
      const value = await fold.run({ type: 'apply', updates: [Y.encodeStateAsUpdate(doc)] });
      results.push({
        name: fixture.name,
        accepted: true,
        shell: value.shell,
        source: value.source,
      });
    } catch {
      results.push({ name: fixture.name, accepted: false });
    } finally {
      fold.close();
      doc.destroy();
    }
  }
  return results;
}
