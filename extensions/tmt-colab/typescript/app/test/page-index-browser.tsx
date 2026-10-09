/** Page-index presentations with deterministic metadata and no real transport effects. */
import { Profiler } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport, type PageSnapshot } from '../src/transport.js';

export const indexNow = Date.UTC(2026, 9, 9, 0, 0);
let root: Root | undefined;
let initialUpdateLabels: string[] | undefined;
export const firstUpdateLabels = () => initialUpdateLabels;
function page(index: number, changes: Partial<PageSnapshot> = {}): PageSnapshot {
  return {
    id: `c${String(index).padStart(7, '0')}-1111-4111-8111-000000000001`,
    title: `Page ${String(index).padStart(3, '0')}`,
    sharing: 'private',
    archived: false,
    source: '<h1>Local page fixture</h1>',
    lastUpdateAtMs: indexNow - index * 60000,
    retentionDays: 30,
    expiresAtMs: null,
    warnings: [],
    ...changes,
  };
}

export async function mountPageIndex(
  size: 'few' | 'many',
  unknownUpdateAtMs: number | null = null,
) {
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('index-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'index-fixture';
    document.body.append(host);
  }
  root?.unmount();
  root = createRoot(host);
  initialUpdateLabels = undefined;
  const container = host;
  const pages =
    size === 'many'
      ? Array.from({ length: 200 }, (_, index) => page(200 - index))
      : [
          page(1, { title: 'Older notes', lastUpdateAtMs: indexNow - 86400000 * 3 }),
          page(3, { title: 'Beta update', sharing: 'public', lastUpdateAtMs: indexNow - 3600000 }),
          page(2, { title: 'Alpha update', sharing: 'link', lastUpdateAtMs: indexNow - 3600000 }),
          page(4, { title: 'Archived notes', archived: true }),
          page(5, { title: '', archived: true, lastUpdateAtMs: unknownUpdateAtMs }),
        ];
  const transport = localTransport('Studio pages', pages);
  const router = createAppRouter({
    ...transport,
    management: {
      read: async () => {
        throw new Error('No management in index fixture');
      },
      prepare: async () => {
        throw new Error('No writes in index fixture');
      },
      send: async () => {
        throw new Error('No sends in index fixture');
      },
      verify: async () => {
        throw new Error('No writes in index fixture');
      },
    },
    spaceHome: async () => ({ title: 'Studio pages', pages }),
  });
  location.hash = '/';
  root.render(
    <Profiler
      id="page-index"
      onRender={() => {
        const times = container.querySelectorAll('.pages > li time');
        if (times.length && initialUpdateLabels === undefined) {
          // Capture the first committed DOM before passive effects can correct it.
          initialUpdateLabels = Array.from(times, (time) => time.textContent ?? '');
        }
      }}
    >
      <RouterProvider router={router} />
    </Profiler>,
  );
}
