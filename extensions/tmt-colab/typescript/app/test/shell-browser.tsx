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

const SHELL = '10000000-0000-4000-8000-000000000001';
const PLAN = '10000000-0000-4000-8000-0000000000a1';
const NOTES = '10000000-0000-4000-8000-0000000000a2';
const OLD = '10000000-0000-4000-8000-0000000000a3';
const GONE = '10000000-0000-4000-8000-0000000000ff';
export const DRAFT = '10000000-0000-4000-8000-0000000000a4';
const item = (n: number) => `20000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
const SUMMARY_AT = Date.UTC(2026, 9, 18, 8, 0, 0);

/** A shell with one item of each kind, a repeated page, a vanished page and an archived page. */
function fullShell(): Shell {
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
      [item(3)]: {
        kind: 'reading',
        pageId: PLAN,
        readingId: '30000000-0000-4000-8000-000000000001',
        note: 'Read before the review.',
      },
      [item(4)]: { kind: 'page', pageId: GONE, note: 'This page was deleted.' },
      [item(5)]: { kind: 'page', pageId: OLD },
      [item(6)]: {
        kind: 'page',
        pageId: NOTES,
        note: `A long note that has to wrap inside its cell without clipping. ${'word '.repeat(60)}`,
      },
    },
    order: [item(1), item(2), item(3), item(4), item(5), item(6)],
    summary: {
      line1: 'Release is on track',
      line2: 'Two reviews still need a decision before Friday.',
      attribution: {
        writerId: '40000000-0000-4000-8000-000000000001',
        timeMs: SUMMARY_AT,
        displayAgent: { name: 'Atlas' },
      },
    },
  };
}

export type ShellCase =
  | 'full'
  | 'empty'
  | 'no-summary'
  | 'pages-failed'
  | 'not-admitted'
  | 'readings'
  | 'readings-failed'
  | 'readings-many';

export const READING = {
  unread: '30000000-0000-4000-8000-0000000000b1',
  read: '30000000-0000-4000-8000-0000000000b2',
  gone: '30000000-0000-4000-8000-0000000000b3',
  pending: '30000000-0000-4000-8000-0000000000b4',
  vanishedPage: '30000000-0000-4000-8000-0000000000b5',
};
const manyPage = (n: number) =>
  `10000000-0000-4000-8000-${(0x1000 + n).toString(16).padStart(12, '0')}`;

/** Reading items on several pages: unread, read, gone from its page, a pending page, a vanished page. */
function readingsShell(many: boolean): Shell {
  if (many) {
    const ids = Array.from({ length: 25 }, (_, n) => item(100 + n));
    return {
      version: 1,
      items: Object.fromEntries(
        ids.map((id, n) => [
          id,
          {
            kind: 'reading' as const,
            pageId: manyPage(n),
            readingId: `30000000-0000-4000-8000-${(0x2000 + n).toString(16).padStart(12, '0')}`,
          },
        ]),
      ),
      order: ids,
    };
  }
  return {
    version: 1,
    items: {
      [item(1)]: { kind: 'reading', pageId: PLAN, readingId: READING.unread },
      [item(2)]: { kind: 'reading', pageId: DRAFT, readingId: READING.read, note: 'Second.' },
      [item(3)]: { kind: 'reading', pageId: NOTES, readingId: READING.gone },
      [item(4)]: { kind: 'reading', pageId: OLD, readingId: READING.pending },
      [item(5)]: { kind: 'reading', pageId: GONE, readingId: READING.vanishedPage },
      [item(6)]: { kind: 'page', pageId: PLAN },
    },
    order: [item(1), item(2), item(3), item(4), item(5), item(6)],
  };
}

declare global {
  interface Window {
    /** Page IDs asked for reading details, in order, and the reading states the fixture serves. */
    detailCalls: string[];
    readStates: Record<string, 'unread' | 'read'>;
  }
}

export function mount(kind: ShellCase = 'full') {
  const rows = [
    { id: SHELL, title: 'Release shell', sharing: 'private' as const },
    {
      id: PLAN,
      title: 'Release plan',
      sharing: 'private' as const,
      lastUpdateAtMs: SUMMARY_AT + 60_000,
      summary: { comments: 3, waiting: false, needsYou: 2 },
    },
    {
      id: NOTES,
      title: 'Design notes',
      sharing: 'private' as const,
      lastUpdateAtMs: SUMMARY_AT - 60_000,
    },
    { id: DRAFT, title: '', titleMissing: true, sharing: 'private' as const },
    {
      id: OLD,
      title: 'Last quarter',
      sharing: 'private' as const,
      archived: true,
      lastUpdateAtMs: SUMMARY_AT - 86_400_000,
    },
  ];
  const full = fullShell();
  const shell: Shell =
    kind === 'empty'
      ? { version: 1, items: {}, order: [] }
      : kind === 'no-summary'
        ? { ...full, summary: undefined }
        : kind === 'readings' || kind === 'readings-failed'
          ? readingsShell(false)
          : kind === 'readings-many'
            ? readingsShell(true)
            : full;
  if (kind === 'readings-many')
    for (let n = 0; n < 25; n++)
      rows.push({ id: manyPage(n), title: `Page ${n}`, sharing: 'private' as const });
  window.detailCalls = [];
  window.readStates = { [READING.unread]: 'unread', [READING.read]: 'read' };
  const detail = (title: string, id: string) => ({
    id,
    title,
    agent: 'Atlas',
    why: 'Read before the review',
    state: window.readStates[id] ?? 'unread',
  });
  const readingDetails =
    kind === 'readings' || kind === 'readings-many' || kind === 'readings-failed'
      ? async (id: string): Promise<ReadingDetails> => {
          window.detailCalls.push(id);
          if (kind === 'readings-failed') throw new Error('offline');
          const ready = (readings: ReturnType<typeof detail>[]): ReadingDetails => ({
            pageId: id,
            state: 'ready',
            fingerprint: 'a'.repeat(64),
            summary: { comments: 0, waiting: false, needsYou: 0 },
            readings,
            needs: [],
          });
          if (id === PLAN) return ready([detail('Decide the scope', READING.unread)]);
          if (id === DRAFT) return ready([detail('Hosting notes', READING.read)]);
          if (id === NOTES) return ready([]);
          return { pageId: id, state: 'pending' };
        }
      : undefined;
  if (kind === 'no-summary') delete shell.summary;
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
