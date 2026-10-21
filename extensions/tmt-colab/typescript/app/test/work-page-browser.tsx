/** An ordinary page that carries work, opened through the real route; never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport, type PageBinding } from '../src/transport.js';
import type { Work } from '../src/work.js';

const attribution = { writerId: '10000000-0000-4000-8000-000000000001', timeMs: 1_000 };
const work = (done: boolean): Work => ({
  version: 1,
  next: 3,
  order: ['T-1', 'T-2'],
  summary: { line1: 'Release notes', line2: 'Drafting', attribution },
  tasks: {
    'T-1': { title: 'Draft', state: done ? 'done' : 'in-progress' },
    'T-2': { title: 'Review', state: 'todo' },
  },
});

declare global {
  interface Window {
    __workPage?: { finish(): void };
  }
}
let root: Root | undefined;

/** `withWork` false is a page that never had work. */
export function mountWorkPage(withWork = true) {
  root?.unmount();
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('work-page-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'work-page-fixture';
    document.body.append(host);
  }
  root = createRoot(host);
  const base = { title: 'Release notes', source: '<h1>Full page</h1>' };
  const binding: PageBinding = {
    subscribe(publish) {
      publish({ ...base, ...(withWork ? { work: work(false) } : {}) });
      window.__workPage = { finish: () => publish({ ...base, work: work(true) }) };
      return () => {};
    },
    async edit() {
      return { merged: false };
    },
    async export() {
      throw new Error('No export in this fixture');
    },
    close() {},
  };
  const transport = localTransport('Review space', [
    { id: 'notes', ...base, sharing: 'private', binding },
  ]);
  root.render(
    <RouterProvider router={createAppRouter({ ...transport, backendName: 'Studio Mac' })} />,
  );
  location.hash = '/pages/notes';
}
