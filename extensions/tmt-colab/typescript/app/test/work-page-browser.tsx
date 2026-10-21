/** An ordinary page that carries work, opened through the real route; never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import type { PageView } from '../src/transport.js';
import { SaveRefused } from '../src/save.js';
import { localTransport, type PageBinding } from '../src/transport.js';
import { applyWorkAction, type Work } from '../src/work.js';

const attribution = { writerId: '10000000-0000-4000-8000-000000000001', timeMs: 1_000 };
const work = (done: boolean, owner = false): Work => ({
  version: 1,
  next: owner ? 4 : 3,
  order: owner ? ['T-1', 'T-2', 'T-3'] : ['T-1', 'T-2'],
  summary: { line1: 'Release notes', line2: 'Drafting', attribution },
  tasks: {
    'T-1': { title: 'Draft', state: done ? 'done' : 'in-progress' },
    'T-2': { title: 'Review', state: 'todo' },
    ...(owner
      ? { 'T-3': { title: 'Copy', state: 'in-review' as const, needsSignoff: true as const } }
      : {}),
  },
});

export interface WorkPageHook {
  finish(): void;
  /** The actions the owner controls sent, in order. */
  calls: { action: string; body: unknown }[];
  /** The next action is refused with this code and message, and nothing changes. */
  refuse?: { code: string; message: string };
}
declare global {
  interface Window {
    __workPage?: WorkPageHook;
  }
}
let root: Root | undefined;

/** `withWork` false is a page that never had work; `owner` true is the owner device, whose
 * binding sends work actions and shows the serve's commit only when it is published back. */
export function mountWorkPage(withWork = true, owner = false) {
  root?.unmount();
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('work-page-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'work-page-fixture';
    document.body.append(host);
  }
  root = createRoot(host);
  const base = { title: 'Release notes', source: '<h1>Full page</h1>' };
  let current = work(false, owner);
  let publishNow: ((value: PageView) => void) | undefined;
  const binding: PageBinding = {
    subscribe(publish) {
      publishNow = publish;
      publish({ ...base, ...(withWork ? { work: current } : {}) });
      window.__workPage = {
        calls: [],
        finish: () => publish({ ...base, work: work(true) }),
      };
      return () => {};
    },
    ...(owner
      ? {
          canWork: () => true,
          async work(action, body) {
            const page = window.__workPage!;
            page.calls.push({ action, body });
            await new Promise((resolve) => setTimeout(resolve, 50));
            if (page.refuse) {
              const { code, message } = page.refuse;
              page.refuse = undefined;
              throw new SaveRefused(code, message);
            }
            current = applyWorkAction(current, action, body, attribution);
            publishNow?.({ ...base, work: current });
            return action === 'task.add' ? { taskId: `T-${current.next - 1}` } : {};
          },
        }
      : {}),
    async edit() {
      return { merged: false };
    },
    async export() {
      throw new Error('No export in this fixture');
    },
    close() {},
  };
  const transport = localTransport('Review space', [
    { id: 'notes', ...base, sharing: 'private', binding },
  ]);
  root.render(
    <RouterProvider router={createAppRouter({ ...transport, backendName: 'Studio Mac' })} />,
  );
  location.hash = '/pages/notes';
}
