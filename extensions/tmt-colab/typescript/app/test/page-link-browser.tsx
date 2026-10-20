/** Owned link-navigation fixture; no production entry imports it. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport } from '../src/transport.js';

export const space = '10000000-0000-4000-8000-000000000001';
export const first = '20000000-0000-4000-8000-000000000001';
export const second = '30000000-0000-4000-8000-000000000001';
let root: Root | undefined;
export function mount(source: string) {
  root?.unmount();
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('page-link-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'page-link-fixture';
    document.body.append(host);
  }
  location.hash = `space=${space}&path=${encodeURIComponent(`/pages/${first}`)}`;
  const transport = localTransport('Link fixture', [
    { id: first, title: 'First page', source, sharing: 'private' },
    {
      id: second,
      title: 'Second page',
      sharing: 'private',
      source:
        '<p>Second page</p><div style="height:1400px"></div><h2 id="details">Target details</h2>',
    },
  ]);
  root = createRoot(host);
  root.render(<RouterProvider router={createAppRouter(transport, space)} />);
}
