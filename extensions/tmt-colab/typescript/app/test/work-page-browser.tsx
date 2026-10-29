/** An ordinary page that carries work, opened through the real route; never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { StrictMode, useState } from 'react';
import { useParentProgress } from '../src/work-stages.js';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import type { PageView } from '../src/transport.js';
import { SaveOutcomeUnknown, SaveRefused } from '../src/save.js';
import { localTransport, type PageBinding } from '../src/transport.js';
import { rememberPages, rememberedPages } from '../src/quick-switcher-source.js';
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
  metadataCaptures?: number;
  holdMetadata?: boolean;
  finishMetadata?(): void;
  /** The actions the owner controls sent, in order. */
  calls: { action: string; body: unknown }[];
  /** The next action is refused with this code and message, and nothing changes. */
  refuse?: { code: string; message: string; childPage?: string };
  refuseLink?: boolean;
  unknownAdd?: boolean;
  unknownLink?: boolean;
  settle?(outcome: 'linked'): void;
  holdAdd?: boolean;
  finishAdd?(): void;
  didAdd?(): void;
}
declare global {
  interface Window {
    __workPage?: WorkPageHook;
    __parentReads?: string[];
    __parentAbort?: AbortSignal;
    __finishParent?: () => void;
    __setSiblingState?: (state: 'todo' | 'done') => void;
    __progressProbe?: {
      requests: { signal: AbortSignal; resolve(): void }[];
      replaceTransport(): void;
      navigate(): void;
    };
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
    | 'child-orphan'
    | 'child-unavailable'
    | 'child-pending'
    | 'child-long'
    | 'child-strict'
    | 'attach',
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
  const base = {
    title:
      scenario === 'child-long'
        ? 'A very long child page title that should be safely truncated without hiding navigation'
        : 'Release notes',
    source: '<h1>Full page</h1>',
  };
  const parentTitle =
    scenario === 'child-long'
      ? 'A very long parent page title that should be safely truncated while remaining accessible'
      : 'Release plan';
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
  let manualState: 'todo' | 'in-progress' | 'done' = 'todo';
  const projection = (): PageView => {
    const values = Object.values(current.tasks);
    const done = values.filter((task) =>
      task.child ? scenario === 'linked-done' : task.state === 'done',
    ).length;
    const unknown = values.filter((task) => task.child && scenario !== 'linked-done').length;
    const todo = values.filter((task) => !task.child && task.state === 'todo').length;
    return {
      ...base,
      state: manualState,
      pageState: manualState,
      ...(withWork ? { work: current } : {}),
      progress: {
        done,
        todo,
        unknown,
        inProgress: values.length - done - todo - unknown,
        total: values.length,
      },
    };
  };
  let publishNow: ((value: PageView) => void) | undefined;
  const binding: PageBinding = {
    subscribe(publish) {
      publishNow = publish;
      publish(projection());
      window.__workPage = {
        calls: [],
        settle: (outcome) => {
          if (outcome === 'linked')
            current = applyWorkAction(
              current,
              'task.link',
              { taskId: 'T-2', pageId: FOLLOWED },
              attribution,
            );
          publish(projection());
        },
        finish: () => {
          current = work(true);
          publish(projection());
        },
      };
      return () => {};
    },
    ...(owner
      ? {
          canMetadata: () => true,
          metadataRevision: async () => {
            window.__workPage!.metadataCaptures = (window.__workPage!.metadataCaptures ?? 0) + 1;
            return `v1:${'a'.repeat(64)}`;
          },
          metadata: async (action, body) => {
            window.__workPage!.calls.push({ action, body });
            if (window.__workPage!.refuse) {
              const refusal = window.__workPage!.refuse;
              window.__workPage!.refuse = undefined;
              throw new SaveRefused(refusal.code, refusal.message);
            }
            if (window.__workPage!.holdMetadata)
              await new Promise<void>((resolve) => {
                window.__workPage!.finishMetadata = resolve;
              });
            manualState = body.state as typeof manualState;
            publishNow?.(projection());
          },
          canWork: () => true,
          async work(action, body) {
            const page = window.__workPage!;
            page.calls.push({ action, body });
            await new Promise((resolve) => setTimeout(resolve, 50));
            if (action === 'task.add' && page.holdAdd)
              await new Promise<void>((resolve) => {
                page.finishAdd = resolve;
              });
            if (action === 'task.add' && page.unknownAdd) {
              page.unknownAdd = false;
              throw new SaveOutcomeUnknown('fixture-operation');
            }
            if (action === 'task.link' && page.unknownLink) {
              page.unknownLink = false;
              throw new SaveOutcomeUnknown('fixture-link');
            }
            if (action === 'task.link' && page.refuseLink) {
              page.refuseLink = false;
              throw new SaveRefused('COLAB_LINK_REFUSED', 'The page already has a parent.');
            }
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
              publishNow?.(projection());
              return { childPage };
            }
            current = applyWorkAction(current, action, body, attribution);
            publishNow?.(projection());
            if (action === 'task.add') page.didAdd?.();
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
  // The child scenario can open its parent: a page of its own with the task the crumb names.
  const parentWork = work(false, true);
  const parentBinding: PageBinding = {
    subscribe(publish) {
      publish({ title: parentTitle, source: '<h1>Parent page</h1>', work: parentWork });
      return () => {};
    },
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
    ...(scenario === 'child-pending'
      ? [
          {
            id: 'welcome',
            title: 'Safe destination',
            source: '<h1>Destination</h1>',
            sharing: 'private' as const,
          },
        ]
      : []),
    ...(scenario?.startsWith('child') && scenario !== 'child-orphan'
      ? [
          {
            id: PARENT,
            title: parentTitle,
            source: '<h1>Parent page</h1>',
            sharing: 'private' as const,
            binding: parentBinding,
          },
        ]
      : []),
  ]);
  const listed = transport.spaceHome.bind(transport);
  window.__parentReads = [];
  const routed = {
    ...transport,
    async readingDetails(id: string, signal: AbortSignal) {
      window.__parentReads!.push(id);
      window.__parentAbort = signal;
      if (scenario === 'child-unavailable') return { pageId: id, state: 'unavailable' as const };
      if (scenario === 'child-pending')
        await new Promise<void>((resolve) => {
          window.__finishParent = resolve;
        });
      return {
        pageId: id,
        state: 'ready' as const,
        fingerprint: 'fixture',
        summary: { comments: 0, waiting: false, needsYou: 0 },
        progress: { done: 3, inProgress: 1, todo: 1, unknown: 0, total: 5 },
        readings: [],
        needs: [],
      };
    },
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
                state: 'done' as const,
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
  if (scenario === 'attach') {
    rememberPages(routed, undefined, [
      { id: 'notes', ...base, sharing: 'private' },
      { id: FOLLOWED, title: 'Legal copy', sharing: 'private' },
      { id: PARENT, title: 'Release plan', sharing: 'private' },
      { id: MADE, title: 'Dashboard', sharing: 'private', pageType: 'shell' },
    ]);
  }
  if (scenario?.startsWith('child')) {
    // Arriving from the list: the list this page was opened from names its parent task.
    const parent = { page: PARENT, task: 'T-3', title: 'Draft the copy' };
    rememberPages(routed, undefined, [
      {
        id: 'notes',
        ...base,
        sharing: 'private',
        summary: { comments: 0, waiting: false, needsYou: 0, parent },
      },
      ...(scenario !== 'child-orphan'
        ? [
            {
              id: PARENT,
              title: parentTitle,
              sharing: 'private' as const,
              state: 'in-progress' as const,
            },
            {
              id: FOLLOWED,
              title: 'Legal copy',
              sharing: 'private' as const,
              state: 'done' as const,
              summary: { comments: 0, waiting: false, needsYou: 0, parent },
            },
          ]
        : []),
    ]);
  }
  window.__setSiblingState = (state) => {
    const pages = rememberedPages(routed, undefined);
    if (pages)
      rememberPages(
        routed,
        undefined,
        pages.map((page) => (page.id === FOLLOWED ? { ...page, state } : page)),
      );
  };
  const app = <RouterProvider router={createAppRouter(routed)} />;
  root.render(scenario === 'child-strict' ? <StrictMode>{app}</StrictMode> : app);
  location.hash = '/pages/notes';
}

/** A test-only hook host for replacement and late-result fencing. */
export function mountParentProgressProbe() {
  root?.unmount();
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('work-page-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'work-page-fixture';
    document.body.append(host);
  }
  root = createRoot(host);
  const requests: { signal: AbortSignal; resolve(): void }[] = [];
  const source = () =>
    ({
      readingDetails(id: string, signal: AbortSignal) {
        return new Promise<import('../src/reading-details.js').ReadingDetails>((resolve) => {
          requests.push({
            signal,
            resolve: () =>
              resolve({
                pageId: id,
                state: 'ready',
                fingerprint: 'fixture',
                summary: { comments: 0, waiting: false, needsYou: 0 },
                progress: { done: 2, inProgress: 1, todo: 1, unknown: 0, total: 4 },
                readings: [],
                needs: [],
              }),
          });
        });
      },
    }) as import('../src/transport.js').PageTransport;
  const first = source();
  const second = source();
  function Probe() {
    const [route, setRoute] = useState({ pageId: 'first', parentId: PARENT, transport: first });
    const progress = useParentProgress(route.transport, route.pageId, route.parentId);
    window.__progressProbe = {
      requests,
      replaceTransport: () => setRoute((old) => ({ ...old, transport: second })),
      navigate: () => setRoute((old) => ({ ...old, pageId: 'second' })),
    };
    return <p role="status">{progress ? `${progress.done}/${progress.total}` : 'unavailable'}</p>;
  }
  root.render(
    <StrictMode>
      <Probe />
    </StrictMode>,
  );
}
