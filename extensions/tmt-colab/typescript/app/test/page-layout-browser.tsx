/** Read-only layout fixture. Never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport } from '../src/transport.js';
import { ReaderApp } from '../src/reader-app.js';
let root: Root | undefined;
export function mount(source: string) {
  root?.unmount();
  document.getElementById('root')!.hidden = true;
  let host = document.getElementById('layout-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'layout-fixture';
    document.body.append(host);
  }
  root = createRoot(host);
  const transport = localTransport('Review space', [
    { id: 'layout', title: 'Release notes', source, sharing: 'private' },
  ]);
  root.render(
    <RouterProvider router={createAppRouter({ ...transport, backendName: 'Studio Mac' })} />,
  );
  location.hash = '/pages/layout';
}

export async function mountReader(source: string) {
  await import('../src/reader-style.css');
  root?.unmount();
  document.getElementById('root')!.hidden = true;
  let host = document.getElementById('layout-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'layout-fixture';
    document.body.append(host);
  }
  root = createRoot(host);
  root.render(<ReaderApp state={{ kind: 'ready', view: { title: 'Reader notes', source } }} />);
}
