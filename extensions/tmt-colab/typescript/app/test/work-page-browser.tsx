/** An ordinary page with sub-pages, opened through the real route; never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import type { PageSummary, PageView } from '../src/transport.js';
import { SaveOutcomeUnknown, SaveRefused } from '../src/save.js';
import { localTransport, type PageBinding, type PageSnapshot } from '../src/transport.js';
import { rememberPages } from '../src/quick-switcher-source.js';
import type { PageState } from '../src/bootstrap.js';
import type { Work } from '../src/work.js';
import { applyWorkAction } from './work-apply.js';

const attribution = { writerId: '10000000-0000-4000-8000-000000000001', timeMs: 1_000 };
const id = (n: number) => `20000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
/** This page's direct sub-pages, a three-level branch, and pages that are free, attached
 * elsewhere, or above this page. */
export const PAGES = {
  remote: id(1),
  invites: id(2),
  machines: id(3),
  draft: id(4),
  machinesDetail: id(11),
  pairing: id(12),
  inviteCopy: id(13),
  qa: id(14),
  columns: id(21),
  inviteText: id(31),
  machineRows: id(41),
  machineFilter: id(42),
  machineEmpty: id(43),
  dataPlan: id(51),
  adminPrototype: id(52),
  launch: id(53),
  elsewhere: id(54),
  made: id(99),
} as const;
const THIS = 'notes';

export interface WorkPageHook {
  /** Every write the panel sent, with the page it went to. */
  calls: { page: string; action: string; body: unknown }[];
  /** The next link or convert on this page is refused with this code and message. */
  refuse?: { code: string; message: string; childPage?: string };
  unknownAdd?: boolean;
  metadataCaptures?: number;
}
declare global {
  interface Window {
    __workPage?: WorkPageHook;
  }
}

type Row = { id: string; title: string; state: PageState; parent?: { page: string; task: string } };
function rows(): Row[] {
  const p = PAGES;
  const under = (page: string, task: number) => ({ page, task: `T-${task}` });
  return [
    { id: p.remote, title: 'Remote pages v2', state: 'in-progress', parent: under(THIS, 1) },
    { id: p.invites, title: 'Invites tab', state: 'done', parent: under(THIS, 3) },
    { id: p.machines, title: 'Machines list', state: 'in-progress', parent: under(THIS, 4) },
    { id: p.draft, title: 'Release notes draft', state: 'done', parent: under(THIS, 5) },
    { id: p.machinesDetail, title: 'Machines detail', state: 'done', parent: under(p.remote, 1) },
    { id: p.pairing, title: 'Pairing flow', state: 'todo', parent: under(p.remote, 2) },
    { id: p.inviteCopy, title: 'Invite copy', state: 'in-progress', parent: under(p.remote, 3) },
    { id: p.qa, title: 'QA pass', state: 'done', parent: under(p.remote, 4) },
    {
      id: p.columns,
      title: 'Device list columns',
      state: 'done',
      parent: under(p.machinesDetail, 1),
    },
    { id: p.inviteText, title: 'Invite text', state: 'done', parent: under(p.invites, 1) },
    { id: p.machineRows, title: 'Machine rows', state: 'done', parent: under(p.machines, 1) },
    { id: p.machineFilter, title: 'Machine filter', state: 'todo', parent: under(p.machines, 2) },
    {
      id: p.machineEmpty,
      title: 'Machine empty state',
      state: 'todo',
      parent: under(p.machines, 3),
    },
    { id: p.dataPlan, title: 'Remote data plan design', state: 'todo' },
    { id: p.adminPrototype, title: 'Remote admin prototype', state: 'todo' },
    { id: p.launch, title: 'Remote launch', state: 'in-progress' },
    { id: p.elsewhere, title: 'Remote copy deck', state: 'todo', parent: under(p.launch, 2) },
  ];
}
const linked = (): Work => ({
  version: 1,
  next: 6,
  order: ['T-1', 'T-2', 'T-3', 'T-4', 'T-5'],
  tasks: {
    'T-1': { title: 'Remote pages v2', state: 'todo', child: PAGES.remote },
    // A plain task is stored but never shown: the panel lists sub-pages only.
    'T-2': { title: 'Hidden plain task', state: 'todo' },
    'T-3': { title: 'Invites tab', state: 'todo', child: PAGES.invites },
    'T-4': { title: 'Machines list', state: 'todo', child: PAGES.machines },
    'T-5': { title: 'Release notes draft', state: 'todo', child: PAGES.draft },
  },
});

let root: Root | undefined;

/**
 * `scenario`: `tree` has four direct sub-pages and a three-level branch; `empty` has none.
 * `owner` true is the owner device: this page's binding and every other page's commit writes, and
 * the owner list follows them as the serve would. `tall` gives the page content taller than the
 * viewport, for the pinned-band checks.
 */
export function mountWorkPage(owner = false, scenario: 'tree' | 'empty' = 'tree', tall = false) {
  root?.unmount();
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('work-page-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'work-page-fixture';
    document.body.append(host);
  }
  root = createRoot(host);
  const hook: WorkPageHook = { calls: [] };
  window.__workPage = hook;
  const list = rows();
  if (scenario === 'empty')
    for (const row of list) if (row.parent?.page === THIS) delete row.parent;
  let current: Work | undefined = scenario === 'tree' ? linked() : undefined;
  let manualState: PageState = 'in-progress';
  const source = tall
    ? `<h1>Release plan</h1>${'<p>Plan paragraph.</p>'.repeat(120)}`
    : '<h1>Release plan</h1>';
  const view = (): PageView => ({
    title: 'Release plan',
    source,
    state: manualState,
    pageState: manualState,
    ...(current ? { work: current } : {}),
  });
  const summary = (row: Row): PageSummary => ({
    id: row.id,
    title: row.title,
    state: row.state,
    sharing: 'private',
    ...(row.parent
      ? { summary: { comments: 0, waiting: false, needsYou: 0, parent: row.parent } }
      : {}),
  });
  const listed = (): PageSummary[] => [
    {
      id: THIS,
      title: 'Release plan',
      state: manualState,
      sharing: 'private',
      summary: {
        comments: 0,
        waiting: false,
        needsYou: 0,
        parent: { page: PAGES.launch, task: 'T-1' },
      },
    },
    ...list.map(summary),
  ];
  let publish: ((value: PageView) => void) | undefined;
  const settle = () => new Promise((resolve) => setTimeout(resolve, 30));
  /** The serve's effect of one of this page's work actions on the owner list. */
  const reflect = (action: string, body: Record<string, unknown>, before: Work | undefined) => {
    if (action === 'task.link') {
      const row = list.find((page) => page.id === body.pageId);
      if (row) row.parent = { page: THIS, task: String(body.taskId) };
    }
    if (action === 'task.remove') {
      const child = before?.tasks[String(body.taskId)]?.child;
      const row = list.find((page) => page.id === child);
      if (row) delete row.parent;
    }
  };
  const ownerControls: Partial<PageBinding> = {
    canWork: () => true,
    canMetadata: () => true,
    metadataRevision: async () => {
      hook.metadataCaptures = (hook.metadataCaptures ?? 0) + 1;
      return `v1:${'a'.repeat(64)}`;
    },
    metadata: async (action, body) => {
      hook.calls.push({ page: THIS, action, body });
      manualState = body.state as PageState;
      publish?.(view());
    },
    async work(action, body) {
      hook.calls.push({ page: THIS, action, body });
      await settle();
      if (action === 'task.add' && hook.unknownAdd) {
        hook.unknownAdd = false;
        throw new SaveOutcomeUnknown('fixture-add');
      }
      if (hook.refuse && (action === 'task.link' || action === 'task.convert')) {
        const { code, message, childPage } = hook.refuse;
        hook.refuse = undefined;
        throw new SaveRefused(code, message, undefined, childPage);
      }
      const before = current;
      const base = current ?? { version: 1, next: 1, order: [], tasks: {} };
      if (action === 'task.convert') {
        // The serve makes the page, titled like the task, then links the task to it.
        const taskId = String(body.taskId);
        list.push({ id: PAGES.made, title: base.tasks[taskId]!.title, state: 'todo' });
        const link = { taskId, pageId: PAGES.made };
        current = applyWorkAction(base, 'task.link', link, attribution);
        reflect('task.link', link, before);
        publish?.(view());
        return { childPage: PAGES.made };
      }
      current = applyWorkAction(base, action, body, attribution);
      reflect(action, body, before);
      publish?.(view());
      return action === 'task.add' ? { taskId: `T-${current.next - 1}` } : {};
    },
  };
  const binding: PageBinding = {
    subscribe(next) {
      publish = next;
      next(view());
      return () => {};
    },
    ...(owner ? ownerControls : {}),
    async edit() {
      return { merged: false };
    },
    async export() {
      throw new Error('No export in this fixture');
    },
    close() {},
  };
  /** Another page, opened only for one owner write: its state, or removing one of its tasks. */
  const otherBinding = (row: Row): PageBinding => ({
    subscribe(next) {
      next({ title: row.title, source: '<p>Other page</p>', state: row.state });
      return () => {};
    },
    canWork: () => owner,
    canMetadata: () => owner,
    metadataRevision: async (shown) => {
      if ((shown.pageState ?? 'todo') !== row.state)
        throw new SaveRefused('COLAB_STALE_BASE', 'Page metadata changed.');
      return `v1:${'b'.repeat(64)}`;
    },
    metadata: async (action, body) => {
      hook.calls.push({ page: row.id, action, body });
      await settle();
      row.state = body.state as PageState;
    },
    async work(action, body) {
      hook.calls.push({ page: row.id, action, body });
      await settle();
      const child = list.find(
        (page) => page.parent?.page === row.id && page.parent.task === body.taskId,
      );
      if (action === 'task.remove' && child) delete child.parent;
      return {};
    },
    async edit() {
      return { merged: false };
    },
    async export() {
      throw new Error('No export in this fixture');
    },
    close() {},
  });
  const transport = localTransport('Review space', [
    { id: THIS, title: 'Release plan', source, sharing: 'private', binding },
  ]);
  const routed = {
    ...transport,
    async spaceHome() {
      return { title: 'Review space', pages: listed() };
    },
    async page(pageId: string, signal?: AbortSignal): Promise<PageSnapshot> {
      if (pageId === THIS) return transport.page(pageId, signal);
      const row = list.find((page) => page.id === pageId);
      if (!row) throw new Error('No such page');
      return {
        ...summary(row),
        source: '<p>Other page</p>',
        pageState: row.state,
        binding: otherBinding(row),
      };
    },
    backendName: 'Studio Mac',
  };
  rememberPages(routed, undefined, listed());
  root.render(<RouterProvider router={createAppRouter(routed)} />);
  location.hash = `/pages/${THIS}`;
}
