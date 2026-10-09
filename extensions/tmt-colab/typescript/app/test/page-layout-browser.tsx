/** Read-only layout fixture. Never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport, type PageBinding, type PageView } from '../src/transport.js';
import type { Projection } from '../src/fold-protocol.js';
import { ReaderApp } from '../src/reader-app.js';
let root: Root | undefined;
let publish: ((view: PageView) => void) | undefined;
let current: PageView;
export function mount(
  source: string,
  attribution?: Pick<Projection, 'originalAuthor' | 'publisherAgent'>,
) {
  root?.unmount();
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('layout-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'layout-fixture';
    document.body.append(host);
  }
  root = createRoot(host);
  current = { title: 'Release notes', source, ...attribution };
  const binding: PageBinding = {
    subscribe(next) {
      publish = next;
      return () => {
        publish = undefined;
      };
    },
    async edit() {
      throw new Error('Read-only fixture');
    },
    async export() {
      throw new Error('Read-only fixture');
    },
    close() {},
  };
  const transport = localTransport('Review space', [
    {
      id: attribution ? 'notes' : 'layout',
      ...current,
      sharing: 'private',
      ...(attribution ? { binding } : {}),
    },
  ]);
  root.render(
    <RouterProvider router={createAppRouter({ ...transport, backendName: 'Studio Mac' })} />,
  );
  // Author fixtures share an existing preview route so the hidden app never opens an unknown page.
  location.hash = attribution ? '/pages/notes' : '/pages/layout';
}
export function updatePublisher(publisherAgent?: string) {
  current = { ...current, publisherAgent };
  publish?.(current);
}

export async function mountReader(
  source: string,
  attribution?: Pick<Projection, 'originalAuthor' | 'publisherAgent'>,
) {
  await import('../src/reader-style.css');
  root?.unmount();
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('layout-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'layout-fixture';
    document.body.append(host);
  }
  root = createRoot(host);
  root.render(
    <ReaderApp
      state={{ kind: 'ready', view: { title: 'Reader notes', source, ...attribution } }}
    />,
  );
}
