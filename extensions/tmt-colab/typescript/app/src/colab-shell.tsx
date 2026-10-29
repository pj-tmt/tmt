import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
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
  type ShellView,
} from '@tmt/browser-ui/shell';
import { knownScope, scopeRows, writeScope } from './project-scope.js';
import { ProjectMenuActionsSlot, type ProjectMenuActions } from './project-menu-actions.js';
import { menuProjects } from './project-pins.js';
import { useQuickSwitcher } from './quick-switcher-context.js';
import { useShellRemoteStatus } from './remote-header.js';
import { SharedTopBar } from './shared-top-bar.js';
import {
  onRememberedPages,
  rememberedPages,
  rememberedProjectPins,
  rememberedProjects,
} from './quick-switcher-source.js';
import { text } from './strings.js';
import type { PageSummary, PageTransport } from './transport.js';

/** The TMT app's slot when Colab runs as its `/colab/` route; the app owns the rail then. */
export const ShellHostSlot = createContext<ShellSlot | null>(null);
const NO_PAGES: readonly PageSummary[] = [];
const NO_PROJECTS: readonly string[] = [];

/**
 * Colab inside the one TMT web shell. The rail comes from Remote's composed descriptions; the
 * module menu is Colab's own: All pages, then each project with its page and needs-you counts,
 * replacing the header's project dropdown, the shell pages, and Colab settings.
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
  // The rail this browser read last paints first, so it matches every other TMT page.
  const [answer, setAnswer] = useState<ShellAnswer | undefined>(rememberedShellAnswer);
  const [actions, setActions] = useState<ProjectMenuActions>();
  // The menu reads nothing itself: it shows the list the routes already read (the space home, a
  // link-opened page's chip read, the quick switcher), so it never adds a list request.
  const listed = useSyncExternalStore(onRememberedPages, () => rememberedPages(transport, space));
  const named = useSyncExternalStore(onRememberedPages, () => rememberedProjects(transport, space));
  const pins = useSyncExternalStore(onRememberedPages, () =>
    rememberedProjectPins(transport, space),
  );
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
    // The menu lists the projects the owner pins, in the owner's order; the scope the list is in
    // stays as a temporary row when it is not pinned, so the current place is always shown.
    const byName = new Map(projects.map((row) => [row.name!, row]));
    const menuNames = menuProjects(
      projects.map((row) => row.name!),
      pins,
    );
    const pinnedRows = menuNames.names.map((name) => byName.get(name)!);
    const temporary =
      shownScope !== undefined && !menuNames.names.includes(shownScope)
        ? byName.get(shownScope)
        : undefined;
    const shells = shown.filter((page) => page.pageType === 'shell');
    const pagePath = (id: string) =>
      router.history.createHref(
        router.buildLocation({ to: '/pages/$pageId', params: { pageId: id } }).publicHref,
      );
    const menu: ShellMenu = {
      title: text.product,
      groups: [
        { items: [item(undefined, all!.count, all!.waiting)] },
        ...(pinnedRows.length || temporary
          ? [
              {
                label: menuNames.automatic ? text.pageList.recentProjects : text.pageList.projects,
                items: [
                  ...pinnedRows.map((row) => item(row.name, row.count, row.waiting)),
                  ...(temporary
                    ? [
                        {
                          ...item(temporary.name, temporary.count, temporary.waiting),
                          note: text.pageList.scopeNotPinned,
                        },
                      ]
                    : []),
                ],
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
                    id: 'manage-projects',
                    label: text.pageList.allProjectsItem,
                    href: '#',
                    current: false,
                  },
                  {
                    id: 'new-project',
                    label: `${text.pageList.newProject}…`,
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
  }, [answer, listed, named, pins, actions, location.pathname, scope]);
  const select = (id: string) => {
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
      <SharedTopBar value={hostSlot.topBarReady()}>{content}</SharedTopBar>
    </HostedShell>
  ) : (
    <OwnShell view={view} select={select}>
      {content}
    </OwnShell>
  );
}

/** Colab's own shell is the root of its common controls: Search opens the quick switcher the
 * route's host owns, and Remote shows the live-session count this page already reads, linking to the
 * Remote settings while a count is unconfirmed. */
function OwnShell({
  view,
  select,
  children,
}: {
  view: ShellView;
  select(id: string): boolean;
  children: ReactNode;
}) {
  const switcher = useQuickSwitcher();
  const owner = useRef(switcher);
  owner.current = switcher;
  const remoteHref = view.rail.main.find((item) => item.id === 'remote')?.href ?? '/remote/';
  const settingsHref = new URL(remoteHref, window.location.href);
  if (settingsHref.pathname === '/remote' || settingsHref.pathname === '/remote/')
    settingsHref.pathname = '/remote/settings';
  const remote = useShellRemoteStatus(settingsHref.href);
  const commonControls = useMemo(
    () => ({
      openSearch(opener: HTMLButtonElement) {
        if (!owner.current) return;
        owner.current.control.current = opener;
        owner.current.activate();
      },
      remote,
    }),
    [remote],
  );
  return (
    <BrowserShell view={view} select={select} commonControls={commonControls}>
      <SharedTopBar value>{children}</SharedTopBar>
    </BrowserShell>
  );
}
