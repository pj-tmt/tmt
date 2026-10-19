/** The mounted router (browser history, short public paths) over a local transport, for navigation proofs. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport, type PageSnapshot } from '../src/transport.js';

let root: Root | undefined;
export const space = 'a'.repeat(32);
const id = (n: number) => `c${String(n).padStart(7, '0')}-1111-4111-8111-000000000001`;
export const pages = [1, 2, 3].map((n) => id(n));

export function mount(start: string, scope?: string) {
  root?.unmount();
  document.getElementById('back-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  document.querySelector('meta[name="tmt-colab-mount"]')?.remove();
  const meta = document.createElement('meta');
  meta.name = 'tmt-colab-mount';
  meta.content = '/r/abcdefghijklmnop/x/colab/';
  document.head.append(meta);
  localStorage.removeItem(`tmt-colab-project-scope-v1:${space}`);
  if (scope) localStorage.setItem(`tmt-colab-project-scope-v1:${space}`, scope);
  history.replaceState(null, '', start);
  const host = document.createElement('div');
  host.id = 'back-fixture';
  document.body.append(host);
  const snapshots: PageSnapshot[] = pages.map((pageId, index) => ({
    id: pageId,
    title: `Page ${index + 1}`,
    sharing: 'private',
    archived: false,
    projects: index < 2 ? ['design'] : [],
    source: `<h1>Body of page ${index + 1}</h1>`,
  }));
  const transport = localTransport('Studio pages', snapshots);
  const spaceHome = transport.spaceHome;
  transport.spaceHome = async () => {
    return {
      ...(await spaceHome()),
      projects: ['design'],
      pages: snapshots.map(({ source: _source, ...summary }) => summary),
      pageIds: pages.map((pageId) => ({ pageId, deleted: false })),
    };
  };
  root = createRoot(host);
  root.render(<RouterProvider router={createAppRouter(transport, space)} />);
}
