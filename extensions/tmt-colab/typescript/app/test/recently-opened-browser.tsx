/** A page route on a space the app knows, over fixed pages: what opening them records on the device. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport, type PageSnapshot } from '../src/transport.js';

export const openedSpace = 'a'.repeat(32);
export const openedPages = [
  '00000000-0000-4000-8000-000000000001',
  '00000000-0000-4000-8000-000000000002',
];
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
  const router = createAppRouter(localTransport('Space', snapshots), openedSpace);
  location.hash = `space=${openedSpace}&path=${encodeURIComponent('/')}`;
  root.render(<RouterProvider router={router} />);
}
