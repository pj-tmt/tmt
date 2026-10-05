/** Isolated screen presentations; no real space, device or transport is opened. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { ReaderApp, type ReaderState } from '../src/reader-app.js';
import { MountedNotice } from '../src/mounted-app.js';
import { localTransport, type PageSnapshot } from '../src/transport.js';

let root: Root | undefined;
const id = 'a19c0460-1111-4111-8111-000000000001';
const source = `<!doctype html><style>body{margin:0;background:#fff;color:#343b58;font:16px/1.7 system-ui}main{max-width:900px;margin:auto;padding:40px}section{padding:36px 0;border-top:1px solid #dde2ee}</style><main><h1>Release notes</h1>${Array.from({ length: 12 }, (_, index) => `<section><h2>Working note ${index + 1}</h2><p>A clear place to read and share work. The header stays close while the document scrolls.</p></section>`).join('')}</main>`;
export type Screen =
  | 'pages'
  | 'archived'
  | 'empty'
  | 'page'
  | 'error'
  | 'not-found'
  | 'mounted-opening'
  | 'mounted-failed'
  | 'reader'
  | 'reader-opening'
  | 'reader-invalid'
  | 'reader-ended'
  | 'reader-failed';

export async function mount(screen: Screen) {
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('chrome-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'chrome-fixture';
    document.body.append(host);
  }
  root?.unmount();
  root = createRoot(host);
  if (screen.startsWith('reader')) {
    await import('../src/reader-style.css');
    const state: ReaderState =
      screen === 'reader'
        ? { kind: 'ready', view: { title: 'Release notes', source } }
        : { kind: screen.slice('reader-'.length) as 'opening' | 'invalid' | 'ended' | 'failed' };
    root.render(<ReaderApp state={state} />);
  } else if (screen.startsWith('mounted-')) {
    const state = screen === 'mounted-opening' ? 'loading' : 'failed';
    root.render(<MountedNotice state={state} />);
  } else {
    const pages: PageSnapshot[] = Array.from(
      { length: screen === 'empty' ? 0 : 24 },
      (_, index) => ({
        id: index === 0 ? id : `b${String(index).padStart(7, '0')}-1111-4111-8111-000000000001`,
        title: index === 1 ? '' : index === 0 ? 'Release notes' : `Working notes ${index + 1}`,
        source,
        sharing: 'private',
        archived: screen === 'archived',
        retentionDays: 30,
        expiresAtMs: null,
        lastUpdateAtMs: null,
        warnings: [],
      }),
    );
    const transport = localTransport('Studio pages', pages);
    const router = createAppRouter({
      ...transport,
      backendName: 'Studio Mac',
      management: {
        read: async () => {
          throw new Error('No management in layout fixture');
        },
        prepare: async () => {
          throw new Error('No writes in layout fixture');
        },
        send: async () => {
          throw new Error('No sends in layout fixture');
        },
        verify: async () => {
          throw new Error('No writes in layout fixture');
        },
      },
      spaceHome: async () => ({ title: 'Studio pages', pages }),
    });
    location.hash =
      screen === 'page'
        ? `/pages/${id}`
        : screen === 'error'
          ? '/blocked'
          : screen === 'not-found'
            ? '/missing'
            : '/';
    root.render(<RouterProvider router={router} />);
  }
}
