/** The Today home with deterministic pages and a fixed clock: no real transport. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport, type PageSnapshot, type PageSummary } from '../src/transport.js';

/** 2026-10-17 09:30 UTC, a Saturday morning; the spec runs in UTC. */
export const todayNow = Date.UTC(2026, 9, 17, 9, 30);
const hour = 3_600_000;
let root: Root | undefined;
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
  summary: { comments, waiting: true, ...(author ? { author } : {}) },
});

export type TodayKind = 'few' | 'many' | 'none' | 'quiet' | 'long';

function pagesFor(kind: TodayKind): PageSummary[] {
  if (kind === 'none') return [];
  if (kind === 'many')
    return [
      page(1, 'Budget review', 30, waiting(3, 'Studio agent')),
      ...Array.from({ length: 14 }, (_, n) =>
        page(n + 2, `Page ${String(n + 1).padStart(2, '0')}`, (n + 1) * 5, {
          summary: { comments: n, waiting: false },
        }),
      ),
    ];
  const quiet = (changes: Partial<PageSummary> = {}) => ({
    summary: { comments: 0, waiting: false },
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
      summary: { comments: 0, waiting: false, author: 'Ada' },
    }),
    page(4, 'Research log', 72, {
      summary: { comments: 5, waiting: false },
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
  const snapshots: PageSnapshot[] = pages.map((p) => ({ ...p, source: '<h1>Fixture</h1>' }));
  const router = createAppRouter({
    ...localTransport('Studio pages', snapshots),
    spaceHome: async () => ({ title: 'Studio pages', pages }),
  });
  location.hash = hash;
  root.render(<RouterProvider router={router} />);
}
