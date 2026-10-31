/** An ordinary page with sub-pages, opened through the real route; never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import type { PageSummary, PageView } from '../src/transport.js';
import { SaveOutcomeUnknown, SaveRefused } from '../src/save.js';
import { localTransport, type PageBinding, type PageSnapshot } from '../src/transport.js';
import { rememberPages } from '../src/quick-switcher-source.js';
import type { PageState } from '../src/bootstrap.js';
import { subpages, type StoredWork } from '../src/work.js';
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
  /** The next attach or create on this page is refused with this code and message. */
  refuse?: { code: string; message: string; childPage?: string };
  /** The next attach on this page has an outcome nobody knows. */
  unknownAttach?: boolean;
  metadataCaptures?: number;
}
declare global {
  interface Window {
    __workPage?: WorkPageHook;
  }
}

type Row = {
  id: string;
  title: string;
  state: PageState;
  parent?: { page: string; index: number };
};
function rows(): Row[] {
  const p = PAGES;
  const under = (page: string, index: number) => ({ page, index });
  return [
    { id: p.remote, title: 'Remote pages v2', state: 'in-progress', parent: under(THIS, 0) },
    { id: p.invites, title: 'Invites tab', state: 'done', parent: under(THIS, 1) },
    { id: p.machines, title: 'Machines list', state: 'in-progress', parent: under(THIS, 2) },
    { id: p.draft, title: 'Release notes draft', state: 'done', parent: under(THIS, 3) },
    { id: p.machinesDetail, title: 'Machines detail', state: 'done', parent: under(p.remote, 0) },
    { id: p.pairing, title: 'Pairing flow', state: 'todo', parent: under(p.remote, 1) },
    { id: p.inviteCopy, title: 'Invite copy', state: 'in-progress', parent: under(p.remote, 2) },
    { id: p.qa, title: 'QA pass', state: 'done', parent: under(p.remote, 3) },
    {
      id: p.columns,
      title: 'Device list columns',
      state: 'done',
      parent: under(p.machinesDetail, 0),
    },
    { id: p.inviteText, title: 'Invite text', state: 'done', parent: under(p.invites, 0) },
    { id: p.machineRows, title: 'Machine rows', state: 'done', parent: under(p.machines, 0) },
    { id: p.machineFilter, title: 'Machine filter', state: 'todo', parent: under(p.machines, 1) },
    {
      id: p.machineEmpty,
      title: 'Machine empty state',
      state: 'todo',
      parent: under(p.machines, 2),
    },
    { id: p.dataPlan, title: 'Remote data plan design', state: 'todo' },
    { id: p.adminPrototype, title: 'Remote admin prototype', state: 'todo' },
    { id: p.launch, title: 'Remote launch', state: 'in-progress' },
    { id: p.elsewhere, title: 'Remote copy deck', state: 'todo', parent: under(p.launch, 1) },
  ];
}
const linked = (): StoredWork => ({
  version: 2,
  order: [PAGES.remote, PAGES.invites, PAGES.machines, PAGES.draft],
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
  let current: StoredWork | undefined = scenario === 'tree' ? linked() : undefined;
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
        parent: { page: PAGES.launch, index: 0 },
      },
    },
    ...list.map(summary),
  ];
  let publish: ((value: PageView) => void) | undefined;
  const settle = () => new Promise((resolve) => setTimeout(resolve, 30));
  /** The serve's effect of this page's list on the owner list: each listed page's parent link. */
  const reflect = () => {
    const order = current ? subpages(current) : [];
    for (const row of list) {
      const index = order.indexOf(row.id);
      if (index >= 0) row.parent = { page: THIS, index };
      else if (row.parent?.page === THIS) delete row.parent;
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
      if (action === 'subpage.attach' && hook.unknownAttach) {
        hook.unknownAttach = false;
        throw new SaveOutcomeUnknown('fixture-attach');
      }
      if (hook.refuse && (action === 'subpage.attach' || action === 'subpage.create')) {
        const { code, message, childPage } = hook.refuse;
        hook.refuse = undefined;
        throw new SaveRefused(code, message, undefined, childPage);
      }
      const base = current ?? { version: 2 as const, order: [] };
      if (action === 'subpage.create') {
        // The serve makes the page, its heading the typed title, and attaches it in one write.
        list.push({ id: PAGES.made, title: String(body.title), state: 'todo' });
        current = applyWorkAction(base, 'subpage.attach', { pageId: PAGES.made }, attribution);
        reflect();
        publish?.(view());
        return { childPage: PAGES.made };
      }
      current = applyWorkAction(base, action, body, attribution);
      reflect();
      publish?.(view());
      return {};
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
  /** Another page, opened only for one owner write: its state, or detaching one of its sub-pages. */
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
      const child = list.find((page) => page.parent?.page === row.id && page.id === body.pageId);
      if (action === 'subpage.detach' && child) delete child.parent;
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
