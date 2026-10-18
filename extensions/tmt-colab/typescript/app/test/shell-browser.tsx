/** Shell fixtures use the real router and isolated Worker; no production transport changes. */
import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import * as Y from 'yjs';
import { createAppRouter } from '../src/router.js';
import { Fold } from '../src/fold.js';
import { ReaderApp } from '../src/reader-app.js';
import type { Shell } from '../src/shell.js';

export function mount() {
  const id = '10000000-0000-4000-8000-000000000001';
  const row = { id, title: 'Release shell', sharing: 'private' as const };
  const shell: Shell = { version: 1, items: {}, order: [] };
  location.hash = `/pages/${id}`;
  const router = createAppRouter({
    backendName: 'Fixture device',
    async spaceHome() {
      return { title: 'Workspace', pages: [row] };
    },
    async page() {
      return { ...row, source: '', pageType: 'shell', shell };
    },
  });
  createRoot(document.getElementById('root')!).render(<RouterProvider router={router} />);
}
export function mountReader() {
  createRoot(document.getElementById('root')!).render(
    <ReaderApp
      state={{
        kind: 'ready',
        pageId: '10000000-0000-4000-8000-000000000001',
        view: {
          source: '',
          title: 'Shared shell',
          pageType: 'shell',
          shell: { version: 1, items: {}, order: [] },
        },
      }}
      canEdit
      edit={async () => {
        throw new Error('A shell has no source edit');
      }}
    />,
  );
}
export async function checkCases(cases: { name: string; document: unknown; action?: string }[]) {
  const results = [];
  for (const fixture of cases) {
    if (fixture.action) continue;
    const doc = new Y.Doc();
    doc.getText('html');
    const meta = doc.getMap('meta');
    doc.transact(() => {
      meta.set('title', 'Shell');
      meta.set('pageType', 'shell');
      meta.set('shell', fixture.document);
    });
    const fold = new Fold();
    try {
      const value = await fold.run({ type: 'apply', updates: [Y.encodeStateAsUpdate(doc)] });
      results.push({
        name: fixture.name,
        accepted: true,
        shell: value.shell,
        source: value.source,
      });
    } catch {
      results.push({ name: fixture.name, accepted: false });
    } finally {
      fold.close();
      doc.destroy();
    }
  }
  return results;
}
