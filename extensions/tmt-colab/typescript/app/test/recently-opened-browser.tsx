/** A page route on a space the app knows, over fixed pages: what opening them records on the device. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { SwitcherSourceStore } from '../src/quick-switcher-source.js';
import { createAppRouter } from '../src/router.js';
import { localTransport, type PageSnapshot, type PageTransport } from '../src/transport.js';

export const openedSpace = 'a'.repeat(32);
export const openedPages = [
  '00000000-0000-4000-8000-000000000001',
  '00000000-0000-4000-8000-000000000002',
];
declare global {
  interface Window {
    __openedTransport?: PageTransport;
  }
}
let root: Root | undefined;

export async function mountOpenedPages() {
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('opened-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'opened-fixture';
    document.body.append(host);
  }
  root?.unmount();
  root = createRoot(host);
  const snapshots: PageSnapshot[] = openedPages.map((id, index) => ({
    id,
    title: `Opened ${index + 1}`,
    sharing: 'private',
    source: `<h1>Opened ${index + 1}</h1>`,
  }));
  const transport = localTransport('Space', snapshots);
  window.__openedTransport = transport;
  const router = createAppRouter(transport, openedSpace);
  location.hash = `space=${openedSpace}&path=${encodeURIComponent('/')}`;
  root.render(<RouterProvider router={router} />);
}

/** What a switcher opened now shows on its first render: read through this module graph, which is the router's. */
export function switcherFirstRender() {
  const initial = new SwitcherSourceStore(window.__openedTransport!, openedSpace).initial();
  return {
    state: initial.state,
    pages: initial.state === 'ready' ? initial.pages.map((row) => row.id) : undefined,
  };
}
