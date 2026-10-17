/** Browser-only fixture over the existing transport port; no native state or services. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { localTransport } from '../src/transport.js';
let root: Root | undefined;
export function mount(prefix: string, collision = true, deleted = false, read?: string) {
  root?.unmount();
  document.getElementById('short-fixture')?.remove();
  document.getElementById('root')?.setAttribute('hidden', '');
  const host = document.createElement('div');
  host.id = 'short-fixture';
  document.body.append(host);
  const space = '10000000-0000-4000-8000-000000000001';
  history.replaceState(
    null,
    '',
    read
      ? `/#space=${space}&path=${encodeURIComponent(`/short/${prefix}`)}&read=${read}`
      : `/#/short/${prefix}`,
  );
  const pages = [
    {
      id: '12345678-0000-4000-8000-000000000001',
      title: 'Original proposal',
      sharing: 'private' as const,
      source:
        '<h1>Original content</h1>' +
        (read
          ? '<tmt-read data-id="10000000-0000-4000-8000-000000000401" data-agent="Agent 1" data-why="Read before deciding"><h2 data-title>Recommended passage</h2><p>The passage.</p></tmt-read>'
          : ''),
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
  root.render(<RouterProvider router={createAppRouter(transport, read ? space : undefined)} />);
}
