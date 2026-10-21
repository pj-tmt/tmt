import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import { useRouter, useRouterState } from '@tanstack/react-router';
import { BrowserShell, HostedShell } from '@tmt/browser-ui/react';
import type { ShellSlot } from '@tmt/browser-ui/shell-host';
import {
  loadShellAnswer,
  rememberedShellAnswer,
  shellAddress,
  shellCount,
  shellRail,
  type ShellAnswer,
  type ShellMenu,
} from '@tmt/browser-ui/shell';
import { knownScope, scopeRows, writeScope } from './project-scope.js';
import { ProjectMenuActionsSlot, type ProjectMenuActions } from './project-menu-actions.js';
import { onRememberedPages, rememberedPages, rememberedProjects } from './quick-switcher-source.js';
import { useQuickSwitcher } from './quick-switcher-context.js';
import { text } from './strings.js';
import type { PageSummary, PageTransport } from './transport.js';

/** The TMT app's slot when Colab runs as its `/colab/` route; the app owns the rail then. */
export const ShellHostSlot = createContext<ShellSlot | null>(null);

const NO_PAGES: readonly PageSummary[] = [];
const NO_PROJECTS: readonly string[] = [];

/**
 * Colab inside the one TMT web shell. The rail comes from Remote's composed descriptions; the
 * module menu is Colab's own: All pages, then each project with its page and needs-you counts,
 * replacing the header's project dropdown, the shell pages, and Colab settings. Search opens the
 * quick switcher.
 */
export function ColabShell({
  transport,
  space,
  children,
}: {
  transport: PageTransport;
  space: string | undefined;
  children: ReactNode;
}) {
  const router = useRouter();
  const hostSlot = useContext(ShellHostSlot);
  const location = useRouterState({ select: (state) => state.location });
  const switcher = useQuickSwitcher();
  // The rail this browser read last paints first, so it matches every other TMT page.
  const [answer, setAnswer] = useState<ShellAnswer | undefined>(rememberedShellAnswer);
  const [actions, setActions] = useState<ProjectMenuActions>();
  // The menu reads nothing itself: it shows the list the routes already read (the space home, a
  // link-opened page's chip read, the quick switcher), so it never adds a list request.
  const listed = useSyncExternalStore(onRememberedPages, () => rememberedPages(transport, space));
  const named = useSyncExternalStore(onRememberedPages, () => rememberedProjects(transport, space));
  useEffect(() => {
    if (hostSlot) return;
    void shellAddress().then(async (address) => {
      const read = address === undefined ? undefined : await loadShellAnswer(address);
      if (read) setAnswer(read);
    });
  }, [hostSlot]);

  const scope =
    location.pathname === '/' ? (location.search as { scope?: string }).scope : undefined;
  const href = (to: string, search?: Record<string, unknown>) =>
    router.history.createHref(router.buildLocation({ to, search }).publicHref);
  const view = useMemo(() => {
    const shown = (listed ?? NO_PAGES).filter((page) => !page.archived);
    const rows = scopeRows(shown, named ?? NO_PROJECTS);
    // A scope the space no longer has shows All pages, as the list itself does.
    const shownScope = knownScope(
      scope,
      rows.slice(1).map((row) => row.name!),
    );
    const item = (name: string | undefined, count: number, waiting: number) => ({
      id: name === undefined ? 'all' : `project:${name}`,
      label: name ?? text.pageList.allPages,
      href: href('/', name === undefined ? {} : { scope: name }),
      total: count,
      count: waiting ? shellCount(waiting) : undefined,
      current: location.pathname === '/' && shownScope === name,
    });
    const [all, ...projects] = rows;
    const shells = shown.filter((page) => page.pageType === 'shell');
    const pagePath = (id: string) =>
      router.history.createHref(
        router.buildLocation({ to: '/pages/$pageId', params: { pageId: id } }).publicHref,
      );
    const menu: ShellMenu = {
      title: text.product,
      groups: [
        { items: [item(undefined, all!.count, all!.waiting)] },
        ...(projects.length
          ? [
              {
                label: text.pageList.projects,
                items: projects.map((row) => item(row.name, row.count, row.waiting)),
              },
            ]
          : []),
        ...(shells.length
          ? [
              {
                label: text.colabSettings.shells,
                items: shells.map((page) => ({
                  id: `shell:${page.id}`,
                  label: page.title,
                  href: pagePath(page.id),
                  current: location.pathname === `/pages/${page.id}`,
                })),
              },
            ]
          : []),
        ...(actions
          ? [
              {
                items: [
                  {
                    id: 'new-project',
                    label: `${text.pageList.newProject}…`,
                    href: '#',
                    current: false,
                  },
                  {
                    id: 'manage-projects',
                    label: `${text.pageList.manageProjects}…`,
                    href: '#',
                    current: false,
                  },
                ],
              },
            ]
          : []),
        {
          items: [
            {
              id: 'colab-settings',
              label: text.colabSettings.title,
              href: href('/settings'),
              current: location.pathname === '/settings',
            },
          ],
        },
      ],
    };
    const origin = window.location.origin;
    // Until the door answers (or with no door, as in the preview), Colab still names itself.
    const self = {
      extension: 'colab',
      rail: { label: text.product, icon: 'file-text', path: href('/'), order: 100 },
      menu: [],
      settings: [],
    };
    return {
      rail: shellRail(answer, { origin, href: window.location.href.split(/[?#]/)[0]! }, self),
      menu,
    };
    // `href` reads the router, which is stable for this shell.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [answer, listed, named, actions, location.pathname, scope]);
  const select = (id: string) => {
    if (id === 'search') return switcher?.activate() ?? false;
    if (id === 'new-project' && actions) return (actions.newProject(), true);
    if (id === 'manage-projects' && actions) return (actions.manage(), true);
    if (id === 'colab-settings') return (void router.navigate({ to: '/settings' }), true);
    if (id.startsWith('shell:')) {
      const pageId = id.slice('shell:'.length);
      void router.navigate({ to: '/pages/$pageId', params: { pageId } });
      return true;
    }
    if (id === 'colab' || id === 'all' || id.startsWith('project:')) {
      // The same remembered scope the list's own project choice writes.
      const name = id.startsWith('project:') ? id.slice('project:'.length) : undefined;
      if (id !== 'colab') writeScope(space, name);
      // On the list a scope changes only the scope, as the list's own choice does: its tab and
      // filters stay, in the same history entry. From elsewhere it opens the list fresh.
      if (location.pathname === '/' && id !== 'colab')
        void router.navigate({
          to: '/',
          search: (previous: Record<string, unknown>) => ({ ...previous, scope: name }),
          replace: true,
        });
      else void router.navigate({ to: '/', search: name === undefined ? {} : { scope: name } });
      return true;
    }
    return false;
  };
  const content = <ProjectMenuActionsSlot value={setActions}>{children}</ProjectMenuActionsSlot>;
  return hostSlot ? (
    <HostedShell slot={hostSlot} menu={view.menu} select={select}>
      {content}
    </HostedShell>
  ) : (
    <BrowserShell view={view} select={select}>
      {content}
    </BrowserShell>
  );
}
