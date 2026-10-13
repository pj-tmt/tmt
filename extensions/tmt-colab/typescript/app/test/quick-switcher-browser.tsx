import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import type { PageSummary, PageTransport } from '../src/transport.js';

let root: Root | undefined;
let reads = 0;
let pageReads: string[] = [];
let failed = false;
let release: (() => void) | undefined;
let pages: PageSummary[] = [];
export const id = (n: number) => `c${String(n).padStart(7, '0')}-1111-4111-8111-000000000001`;

/** Deterministic data only; production parser, ranking, hook, routing and renderer stay real. */
export async function mount(
  mode: 'home' | 'page' | 'empty' | 'cold-failed' | 'cold-loading' = 'home',
) {
  root?.unmount();
  reads = 0;
  pageReads = [];
  failed = mode === 'cold-failed';
  release = undefined;
  pages =
    mode === 'empty'
      ? []
      : [
          {
            id: id(1),
            title: 'Alpha brief',
            sharing: 'private',
            labels: ['design'],
            lastUpdateAtMs: 30,
            summary: { comments: 0, waiting: false, author: 'Ada' },
          },
          {
            id: id(2),
            title: 'Beta review',
            sharing: 'private',
            labels: ['ops'],
            lastUpdateAtMs: 20,
            summary: { comments: 1, waiting: true, author: 'Pat' },
          },
          {
            id: id(3),
            title: 'Long title <img src=x onerror="window.executed=true"> kept as text',
            sharing: 'private',
            labels: ['design', 'a long label to wrap at narrow widths'],
            lastUpdateAtMs: 10,
            summary: { comments: 0, waiting: false, author: 'Ada' },
          },
          {
            id: id(4),
            title: 'Archived work',
            sharing: 'private',
            archived: true,
            labels: ['ops'],
          },
        ];
  const gate =
    mode === 'cold-loading'
      ? new Promise<void>((resolve) => {
          release = resolve;
        })
      : undefined;
  const transport: PageTransport = {
    backendName: 'Fixture device',
    async spaceHome() {
      reads++;
      await gate;
      if (failed) throw new Error('Fixture list read refused');
      return { title: 'Studio pages', pages };
    },
    async page(pageId) {
      pageReads.push(pageId);
      const row = pages.find((row) => row.id === pageId);
      if (!row) throw new Error('Fixture page missing');
      return { ...row, source: '<p>Page content</p><input aria-label="Author link editor">' };
    },
  };
  location.hash = mode === 'home' || mode === 'empty' ? '/' : `/pages/${id(1)}`;
  const router = createAppRouter(transport);
  root = createRoot(document.getElementById('root')!);
  root.render(<RouterProvider router={router} />);
}
export function proof() {
  return { reads, pageReads: [...pageReads] };
}
export function failRefresh() {
  failed = true;
}
export function finishRead() {
  release?.();
}
export function remove(pageId: string) {
  pages = pages.filter((row) => row.id !== pageId);
}
