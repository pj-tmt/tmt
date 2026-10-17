/** The Today home with deterministic pages and a fixed clock: no real transport. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport, type PageSnapshot, type PageSummary } from '../src/transport.js';

/** 2026-10-17 09:30 UTC, a Saturday morning; the spec runs in UTC. */
export const todayNow = Date.UTC(2026, 9, 17, 9, 30);
const hour = 3_600_000;
let root: Root | undefined;
let homePages: PageSummary[] = [];
let homeReads = 0;

/** Replace only the next owner-list response; the router and mounted view stay alive. */
export function nextHome(kind: TodayKind) {
  homePages = pagesFor(kind);
}
export function homeReadCount() {
  return homeReads;
}
const id = (n: number) => `d${String(n).padStart(7, '0')}-1111-4111-8111-000000000001`;
function page(n: number, title: string, ago: number, changes: Partial<PageSummary> = {}) {
  return {
    id: id(n),
    title,
    sharing: 'private' as const,
    archived: false,
    lastUpdateAtMs: todayNow - ago * hour,
    retentionDays: 30,
    expiresAtMs: null,
    warnings: [],
    labels: [] as string[],
    ...changes,
  };
}
const waiting = (comments: number, author?: string) => ({
  summary: { comments, waiting: true, needsYou: 1, ...(author ? { author } : {}) },
});

export type TodayKind =
  | 'few'
  | 'many'
  | 'none'
  | 'quiet'
  | 'long'
  | 'mixed'
  | 'checking'
  | 'partial';

function pagesFor(kind: TodayKind): PageSummary[] {
  if (kind === 'none') return [];
  if (kind === 'checking') return [page(1, 'Pages are being checked', 1)];
  if (kind === 'partial')
    return [
      page(1, 'Release decisions', 1, { summary: { comments: 1, waiting: true, needsYou: 3 } }),
      page(2, 'Still checking', 2),
    ];
  if (kind === 'mixed')
    return [
      page(1, 'Release decisions', 1, {
        summary: { comments: 1, waiting: true, needsYou: 4, author: 'Agent 1' },
      }),
      page(2, 'One reading', 2, {
        summary: { comments: 0, waiting: true, needsYou: 1, author: 'Agent 2' },
      }),
      page(3, 'Finished notes', 3, { summary: { comments: 2, waiting: false, needsYou: 0 } }),
    ];

  if (kind === 'many')
    return [
      page(1, 'Budget review', 30, waiting(3, 'Studio agent')),
      ...Array.from({ length: 14 }, (_, n) =>
        page(n + 2, `Page ${String(n + 1).padStart(2, '0')}`, (n + 1) * 5, {
          summary: { comments: n, waiting: false, needsYou: 0 },
        }),
      ),
    ];
  const quiet = (changes: Partial<PageSummary> = {}) => ({
    summary: { comments: 0, waiting: false, needsYou: 0 },
    ...changes,
  });
  if (kind === 'quiet')
    return [
      page(1, 'Notes from standup', 2, quiet({ labels: ['team'] })),
      page(2, 'Research log', 72, quiet()),
    ];
  if (kind === 'long')
    return [
      page(
        1,
        'A deliberately long title for the quarterly planning review that keeps going past any sensible line width so the chapter has to wrap it',
        30,
        { ...waiting(12, 'Studio agent'), labels: [] },
      ),
      page(2, 'Notes from standup', 2, quiet()),
    ];
  return [
    page(1, 'Budget review', 30, {
      ...waiting(3, 'Studio agent'),
      labels: ['finance', 'q4 plan'],
    }),
    page(2, 'Vendor contract', 50, waiting(1)),
    page(3, 'Notes from standup', 2, {
      summary: { comments: 0, waiting: false, needsYou: 0, author: 'Ada' },
    }),
    page(4, 'Research log', 72, {
      summary: { comments: 5, waiting: false, needsYou: 0 },
      labels: ['research', 'design'],
    }),
    // No summary yet: the chapter shows only what the list already knows.
    page(5, 'Old draft', 120),
    page(6, 'Archived spec', 200, { archived: true }),
  ];
}

export async function mountToday(kind: TodayKind = 'few', hash = '/') {
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('today-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'today-fixture';
    document.body.append(host);
  }
  root?.unmount();
  root = createRoot(host);
  const pages = pagesFor(kind);
  homePages = pages;
  homeReads = 0;
  const snapshots: PageSnapshot[] = pages.map((p) => ({ ...p, source: '<h1>Fixture</h1>' }));
  const router = createAppRouter({
    ...localTransport('Studio pages', snapshots),
    spaceHome: async () => {
      homeReads++;
      return { title: 'Studio pages', pages: homePages };
    },
  });
  location.hash = hash;
  root.render(<RouterProvider router={router} />);
}
