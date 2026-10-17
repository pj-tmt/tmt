/** The page list with label writes recorded in the window: deterministic data, no real transport. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { ManagementError, type ManagementPort } from '../src/management.js';
import type { LabelChange } from '../src/labels.js';
import { localTransport, type PageSnapshot, type PageSummary } from '../src/transport.js';

export const listNow = Date.UTC(2026, 9, 9, 0, 0);
declare global {
  interface Window {
    __labelRequests: { page: string; changes: LabelChange[] }[];
    /** Fail the label request with this index (0-based) once, as a lost connection. */
    __failLabelRequest?: number;
  }
}
let root: Root | undefined;
const id = (n: number) => `c${String(n).padStart(7, '0')}-1111-4111-8111-000000000001`;
function page(n: number, title: string, labels: string[], changes: Partial<PageSummary> = {}) {
  return {
    id: id(n),
    title,
    sharing: 'private' as const,
    archived: false,
    lastUpdateAtMs: listNow - n * 3600_000,
    retentionDays: 30,
    expiresAtMs: null,
    warnings: [],
    labels,
    ...changes,
  };
}

export async function mountPageList(size: 'few' | 'bulk' | 'none' | 'unmanaged' = 'few') {
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('list-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'list-fixture';
    document.body.append(host);
  }
  root?.unmount();
  root = createRoot(host);
  window.__labelRequests = [];
  window.__failLabelRequest = undefined;
  const state: { pages: PageSummary[] } = {
    pages:
      size === 'bulk'
        ? [
            ...Array.from({ length: 130 }, (_, n) =>
              page(n + 1, `Bulk ${String(n + 1).padStart(3, '0')}`, ['old']),
            ),
            page(131, 'Bulk archived', ['old', 'z'], { archived: true }),
          ]
        : size === 'none'
          ? []
          : [
              page(1, 'Alpha plan', ['design', 'q4 plan'], {
                summary: { comments: 3, waiting: true, author: 'Studio agent' },
              }),
              page(2, 'Beta review', ['design'], {
                summary: { comments: 1, waiting: false, author: 'Ada' },
              }),
              // No summary yet: the row shows only what the list already knows.
              page(3, 'Gamma notes', []),
              page(4, 'Delta ops', ['ops'], { summary: { comments: 0, waiting: false } }),
              page(5, 'Archived spec', ['design'], { archived: true }),
            ],
  };
  const snapshots: PageSnapshot[] = state.pages.map((p) => ({ ...p, source: '<h1>Fixture</h1>' }));
  const transport = localTransport('Studio pages', snapshots);
  const management: ManagementPort = {
    read: async () => {
      throw new Error('No management view in the list fixture');
    },
    prepare: async () => {
      throw new Error('No log operations in the list fixture');
    },
    send: async () => {
      throw new Error('No log operations in the list fixture');
    },
    verify: async () => {
      throw new Error('No log operations in the list fixture');
    },
    setLabels: async (changes) => {
      const index = window.__labelRequests.length;
      window.__labelRequests.push({ page: changes[0].pageId, changes: [...changes] });
      if (window.__failLabelRequest === index) {
        window.__failLabelRequest = undefined;
        throw new ManagementError('UNKNOWN');
      }
      state.pages = state.pages.map((p) => {
        const change = changes.find((c) => c.pageId === p.id);
        return change ? { ...p, labels: [...change.labels] } : p;
      });
    },
  };
  const router = createAppRouter({
    ...transport,
    ...(size === 'unmanaged' ? {} : { management }),
    spaceHome: async () => ({ title: 'Studio pages', pages: state.pages }),
  });
  location.hash = '/all';
  root.render(<RouterProvider router={router} />);
}
