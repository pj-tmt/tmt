/** Browser-only fixture over the existing transport port; no production imports. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport } from '../src/transport.js';
let root: Root | undefined;
export function mount(prefix: string, collision = true, deleted = false, publicPath = false) {
  root?.unmount();
  document.getElementById('short-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('div');
  host.id = 'short-fixture';
  document.body.append(host);
  if (publicPath) {
    const meta = document.createElement('meta');
    meta.name = 'tmt-colab-mount';
    meta.content = '/r/abcdefghijklmnop/x/colab/';
    document.head.append(meta);
  }
  history.replaceState(null, '', publicPath ? `/p/${prefix}` : `/#/short/${prefix}`);
  const pages = [
    {
      id: '12345678-0000-4000-8000-000000000001',
      title: 'Original proposal',
      sharing: 'private' as const,
      source: '<h1>Original content</h1>',
    },
  ];
  if (collision)
    pages.push({
      id: '12345678-1000-4000-8000-000000000002',
      title: 'Later proposal',
      sharing: 'private',
      source: '<h1>Later content</h1>',
    });
  const pageIds = pages.map((page, index) => ({
    pageId: page.id,
    deleted: deleted && index === 0,
  }));
  const transport = localTransport(
    'Colab',
    pages.filter((_page, index) => !deleted || index !== 0),
  );
  const spaceHome = transport.spaceHome;
  transport.spaceHome = async () => ({ ...(await spaceHome()), pageIds });
  root = createRoot(host);
  root.render(
    <RouterProvider router={createAppRouter(transport, publicPath ? 'a'.repeat(32) : undefined)} />,
  );
}
