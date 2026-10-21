/** An ordinary page that carries work, opened through the real route; never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import type { PageView } from '../src/transport.js';
import { SaveRefused } from '../src/save.js';
import { localTransport, type PageBinding } from '../src/transport.js';
import { rememberPages } from '../src/quick-switcher-source.js';
import type { Work } from '../src/work.js';
import { applyWorkAction } from './work-apply.js';

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
  refuse?: { code: string; message: string; childPage?: string };
}
declare global {
  interface Window {
    __workPage?: WorkPageHook;
  }
}
let root: Root | undefined;
const FOLLOWED = '20000000-0000-4000-8000-000000000002';
const MADE = '20000000-0000-4000-8000-000000000009';
const PARENT = '30000000-0000-4000-8000-000000000003';

/** `withWork` false is a page that never had work; `owner` true is the owner device, whose
 * binding sends work actions and shows the serve's commit only when it is published back.
 * `scenario`: `linked-done` has T-2 follow a page whose list summary says done; `linked-missing`
 * follows one the list does not name; `summary-only` is a summary with no task; `empty` is a stored
 * value with neither (every task removed); `external` has T-2 tracked by an external issue. `child` is
 * this page's list row saying a task of `PARENT` ("Release plan") follows it; `child-orphan` says so
 * while the list does not name that parent. */
export function mountWorkPage(
  withWork = true,
  owner = false,
  scenario?:
    | 'linked-done'
    | 'linked-missing'
    | 'summary-only'
    | 'external'
    | 'empty'
    | 'child'
    | 'child-orphan',
) {
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
  if (scenario === 'empty') current = { version: 1, next: 1, order: [], tasks: {} };
  else if (scenario === 'summary-only')
    current = {
      version: 1,
      next: 1,
      order: [],
      tasks: {},
      summary: { line1: 'Release notes', line2: 'Nothing planned yet', attribution },
    };
  else if (scenario === 'external')
    current = {
      ...current,
      tasks: { ...current.tasks, 'T-2': { ...current.tasks['T-2'], externalId: '#7' } },
    };
  else if (scenario?.startsWith('linked'))
    current = {
      ...current,
      tasks: { ...current.tasks, 'T-2': { ...current.tasks['T-2'], child: FOLLOWED } },
    };
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
              const { code, message, childPage } = page.refuse;
              page.refuse = undefined;
              throw new SaveRefused(code, message, undefined, childPage);
            }
            if (action === 'task.convert') {
              // The serve makes the page, then links the task to it.
              const childPage = MADE;
              current = applyWorkAction(
                current,
                'task.link',
                { taskId: (body as { taskId: string }).taskId, pageId: childPage },
                attribution,
              );
              publishNow?.({ ...base, work: current });
              return { childPage };
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
  const listed = transport.spaceHome.bind(transport);
  const routed = {
    ...transport,
    // The owner list names the followed page, with the stage its summary says.
    async spaceHome() {
      const home = await listed();
      return scenario === 'linked-done'
        ? {
            ...home,
            pages: [
              ...home.pages,
              {
                id: FOLLOWED,
                title: 'Legal copy',
                sharing: 'private' as const,
                summary: {
                  comments: 0,
                  waiting: false,
                  needsYou: 0,
                  work: { stage: 'done' as const, done: 1, total: 1, signoff: 0, working: [] },
                },
              },
            ],
          }
        : home;
    },
    backendName: 'Studio Mac',
  };
  if (scenario === 'child' || scenario === 'child-orphan') {
    // Arriving from the list: the list this page was opened from names its parent task.
    const parent = { page: PARENT, task: 'T-3' };
    rememberPages(routed, undefined, [
      {
        id: 'notes',
        ...base,
        sharing: 'private',
        summary: { comments: 0, waiting: false, needsYou: 0, parent },
      },
      ...(scenario === 'child'
        ? [{ id: PARENT, title: 'Release plan', sharing: 'private' as const }]
        : []),
    ]);
  }
  root.render(<RouterProvider router={createAppRouter(routed)} />);
  location.hash = '/pages/notes';
}
