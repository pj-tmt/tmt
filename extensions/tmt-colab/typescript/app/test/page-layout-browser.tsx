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
/** One linked sub-page, so the status band has a list to open. */
const SUBPAGE_WORK: NonNullable<PageView['work']> = {
  version: 2,
  order: ['20000000-0000-4000-8000-000000000002'],
};
export function mount(
  source: string,
  attribution?: Pick<Projection, 'originalAuthor' | 'publisherAgent'>,
  header: { title?: string; deviceName?: string } = { deviceName: 'Studio Mac' },
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
  current = { title: header.title ?? 'Release notes', source, work: SUBPAGE_WORK, ...attribution };
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
    <RouterProvider router={createAppRouter({ ...transport, backendName: header.deviceName })} />,
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
  attribution?: Pick<PageView, 'originalAuthor' | 'publisherAgent' | 'editLocks'>,
) {
  await import('@tmt/browser-ui/static.css');
  await import('../src/reader-style.css');
  root?.unmount();
  // The shipped reader has no TMT shell: drop the app's hidden one, which would still match
  // `body:has(.tmt-ui-shell)` and take away the body's inset for the fixed header.
  document.getElementById('root')!.replaceChildren();
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
      state={{
        kind: 'ready',
        view: { title: 'Reader notes', source, work: SUBPAGE_WORK, ...attribution },
      }}
    />,
  );
}
