/** Two local pages and a hold on opening them, to watch what shows while a page is entered again. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport, type PageSnapshot } from '../src/transport.js';

declare global {
  interface Window {
    /** Page IDs the transport was asked to open, in order. */
    pageOpens: string[];
    /** While set, opening a page waits for it (the open is "slow"). */
    holdPages?: Promise<void>;
    /** When set, a held open that is released fails with this message, as a real open failure. */
    failPages?: string;
  }
}
const ID = {
  alpha: 'c0000001-1111-4111-8111-000000000001',
  beta: 'c0000002-1111-4111-8111-000000000001',
};
let root: Root | undefined;

export function mount() {
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('reentry-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'reentry-fixture';
    document.body.append(host);
  }
  root?.unmount();
  root = createRoot(host);
  window.pageOpens = [];
  const snapshots: PageSnapshot[] = [
    ['alpha', 'Alpha plan'],
    ['beta', 'Beta review'],
  ].map(([key, title], n) => ({
    id: ID[key as keyof typeof ID],
    title,
    sharing: 'private' as const,
    archived: false,
    lastUpdateAtMs: Date.UTC(2026, 9, 9) - n * 3600_000,
    retentionDays: 30,
    expiresAtMs: null,
    warnings: [],
    projects: [],
    source: `<h1>${title}</h1><p>Body of ${title}.</p>`,
  }));
  const base = localTransport('Studio pages', snapshots);
  const router = createAppRouter({
    ...base,
    async page(id, signal) {
      window.pageOpens.push(id);
      await window.holdPages;
      if (window.failPages) throw new Error(window.failPages);
      return base.page(id, signal);
    },
  });
  location.hash = '/';
  root.render(<RouterProvider router={router} />);
}
export { ID };
