/** The page list with project writes recorded in the window: deterministic data, no real transport. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import { ManagementError, type ManagementPort, type Selection } from '../src/management.js';
import type { ProjectChange, ProjectManage } from '../src/projects.js';
import { localTransport, type PageSnapshot, type PageSummary } from '../src/transport.js';

export const listNow = Date.UTC(2026, 9, 9, 0, 0);
declare global {
  interface Window {
    __projectRequests: { page: string; changes: ProjectChange[] }[];
    /** Every `project.manage` request, in order. */
    __projectManage: ProjectManage[];
    /** Fail the page request with this index (0-based) once, as a lost connection. */
    __failProjectRequest?: number;
    /** Fail the next `project.manage` request once. */
    __failManage?: boolean;
    /** Hold the next `project.manage` request until `__releaseManage` runs. */
    __holdManage?: boolean;
    __releaseManage?: () => void;
    /** Every archive and delete statement the owner accepted, in order. */
    __pageOperations: { operation: string; pageId: string }[];
    /** `refuse`: the owner refuses the next statement; `unverified`: its log cannot be read back. */
    __operationMode?: 'refuse' | 'unverified';
    /** What `window.open` was asked for, since the fixture replaces it. */
    __opened: { url: string; target: string; features: string }[];
  }
}
let root: Root | undefined;
const id = (n: number) => `c${String(n).padStart(7, '0')}-1111-4111-8111-000000000001`;
function page(n: number, title: string, projects: string[], changes: Partial<PageSummary> = {}) {
  return {
    id: id(n),
    title,
    sharing: 'private' as const,
    archived: false,
    lastUpdateAtMs: listNow - n * 3600_000,
    retentionDays: 30,
    expiresAtMs: null,
    warnings: [],
    projects,
    ...changes,
  };
}

export async function mountPageList(
  size: 'few' | 'bulk' | 'none' | 'unmanaged' | 'long' = 'few',
  hash = '/',
) {
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('list-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'list-fixture';
    document.body.append(host);
  }
  root?.unmount();
  root = createRoot(host);
  window.__projectRequests = [];
  window.__projectManage = [];
  window.__failProjectRequest = undefined;
  window.__failManage = undefined;
  window.__pageOperations = [];
  window.__operationMode = undefined;
  window.__opened = [];
  window.open = (url, target, features) => {
    window.__opened.push({ url: String(url), target: target ?? '', features: features ?? '' });
    return null;
  };
  const state: { pages: PageSummary[]; projects: Set<string> } = {
    projects: new Set(),
    pages:
      size === 'bulk'
        ? [
            ...Array.from({ length: 130 }, (_, n) =>
              page(n + 1, `Bulk ${String(n + 1).padStart(3, '0')}`, ['old']),
            ),
            page(131, 'Bulk archived', ['old', 'z'], { archived: true }),
          ]
        : size === 'none'
          ? []
          : [
              // The longest names the owner admits: 32 wide characters, 32 letters, a long creator.
              ...(size === 'long'
                ? [
                    page(6, 'Long names', ['漢'.repeat(32), 'W'.repeat(32)], {
                      summary: {
                        comments: 0,
                        waiting: false,
                        needsYou: 0,
                        author: 'W'.repeat(120),
                      },
                    }),
                  ]
                : []),
              page(1, 'Alpha plan', ['design', 'q4 plan'], {
                summary: { comments: 3, waiting: true, needsYou: 1, author: 'Studio agent' },
              }),
              page(2, 'Beta review', ['design'], {
                summary: { comments: 1, waiting: false, needsYou: 0, author: 'Ada' },
              }),
              // No summary yet: the row shows only what the list already knows.
              page(3, 'Gamma notes', []),
              page(4, 'Delta ops', ['ops'], {
                summary: { comments: 0, waiting: false, needsYou: 0 },
              }),
              page(5, 'Archived spec', ['design'], { archived: true }),
            ],
  };
  for (const p of state.pages) for (const name of p.projects ?? []) state.projects.add(name);
  const snapshots: PageSnapshot[] = state.pages.map((p) => ({ ...p, source: '<h1>Fixture</h1>' }));
  const transport = localTransport('Studio pages', snapshots);
  const management: ManagementPort = {
    // The list's archive and delete run the real four steps; the fixture answers each in memory.
    read: async (pageId) => ({ page: { pageId } }) as never,
    prepare: async (_view, selection) => ({ selection }) as never,
    send: async (pending) => {
      const { operation, value } = (pending as never as { selection: Selection }).selection as {
        operation: string;
        value: { pageId: string };
      };
      if (window.__operationMode === 'refuse') {
        window.__operationMode = undefined;
        throw new ManagementError('DENIED');
      }
      window.__pageOperations.push({ operation, pageId: value.pageId });
      state.pages =
        operation === 'page.delete'
          ? state.pages.filter((p) => p.id !== value.pageId)
          : state.pages.map((p) => (p.id === value.pageId ? { ...p, archived: true } : p));
      return {} as never;
    },
    verify: async (_pending, _ack) => {
      if (window.__operationMode === 'unverified') {
        window.__operationMode = undefined;
        throw new Error('The signed log is not readable yet');
      }
      return {} as never;
    },
    setProjects: async (changes) => {
      const index = window.__projectRequests.length;
      window.__projectRequests.push({ page: changes[0].pageId, changes: [...changes] });
      if (window.__failProjectRequest === index) {
        window.__failProjectRequest = undefined;
        throw new ManagementError('UNKNOWN');
      }
      for (const change of changes) for (const name of change.projects) state.projects.add(name);
      state.pages = state.pages.map((p) => {
        const change = changes.find((c) => c.pageId === p.id);
        return change ? { ...p, projects: [...change.projects] } : p;
      });
    },
    manageProject: async (change) => {
      window.__projectManage.push(change);
      if (window.__holdManage) {
        window.__holdManage = undefined;
        await new Promise<void>((resolve) => {
          window.__releaseManage = resolve;
        });
      }
      if (window.__failManage) {
        window.__failManage = undefined;
        throw new ManagementError('UNKNOWN');
      }
      if (change.action === 'create') state.projects.add(change.name);
      else {
        const target = change.action === 'rename' ? change.to : undefined;
        state.projects.delete(change.name);
        if (target !== undefined) state.projects.add(target);
        state.pages = state.pages.map((p) =>
          p.projects?.includes(change.name)
            ? {
                ...p,
                projects: [
                  ...new Set(
                    p.projects.flatMap((name) =>
                      name !== change.name ? [name] : target === undefined ? [] : [target],
                    ),
                  ),
                ].sort(),
              }
            : p,
        );
      }
    },
  };
  // Set before the router reads its history, so a redirect in the first load is not overwritten.
  location.hash = hash;
  const router = createAppRouter({
    ...transport,
    ...(size === 'unmanaged' ? {} : { management }),
    spaceHome: async () => ({
      title: 'Studio pages',
      projects: [...state.projects].sort(),
      pages: state.pages,
    }),
  });
  root.render(<RouterProvider router={router} />);
}
