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
import { BrowserAction, BrowserDialog, BrowserShell, HostedShell } from '@tmt/browser-ui/react';
import type { ShellSlot } from '@tmt/browser-ui/shell-host';
import {
  loadShellAnswer,
  rememberedShellAnswer,
  shellAddress,
  shellCount,
  shellRail,
  type ShellAnswer,
  type ShellMenu,
  type ShellMenuActionEvent,
  type ShellMenuReorderEvent,
  type ShellView,
} from '@tmt/browser-ui/shell';
import { knownScope, scopeRows, writeScope } from './project-scope.js';
import { ProjectMenuActionsSlot, type ProjectMenuActions } from './project-menu-actions.js';
import { useShortcuts } from './use-shortcuts.js';
import { ShortcutDialog } from './shortcut-dialogs.js';
import { listPages } from './page-list-filter.js';
import type { Shortcut } from './shortcuts.js';
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
 * module menu is Colab's own: All pages, local Shortcuts, dashboards and Colab settings.
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
  const shortcuts = useShortcuts(space);
  const [shortcutDialog, setShortcutDialog] = useState<{
    mode: 'manage' | 'edit' | 'create';
    id?: string;
  }>();
  const [deleteShortcut, setDeleteShortcut] = useState<string>();
  const [shortcutError, setShortcutError] = useState<string>();
  // The menu reads nothing itself: it shows the list the routes already read (the space home, a
  // link-opened page's chip read, the quick switcher), so it never adds a list request.
  const listed = useSyncExternalStore(onRememberedPages, () => rememberedPages(transport, space));
  const named = useSyncExternalStore(onRememberedPages, () => rememberedProjects(transport, space));
  const pins = useSyncExternalStore(onRememberedPages, () =>
    rememberedProjectPins(transport, space),
  );
  useEffect(() => {
    if (!shortcuts.store || !pins) return;
    try {
      shortcuts.store.migrate(pins);
      setShortcutError(undefined);
    } catch {
      setShortcutError(text.shortcuts.saveFailed);
    }
  }, [shortcuts.store, pins]);
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
      current:
        location.pathname === '/' &&
        shownScope === name &&
        !(location.search as { shortcut?: string }).shortcut,
    });
    const [all] = rows;
    const shells = shown.filter((page) => page.pageType === 'shell');
    const pagePath = (id: string) =>
      router.history.createHref(
        router.buildLocation({ to: '/pages/$pageId', params: { pageId: id } }).publicHref,
      );
    const menu: ShellMenu = {
      title: text.product,
      groups: [
        { items: [item(undefined, all!.count, all!.waiting)] },
        ...(shortcuts.store
          ? [
              {
                id: 'shortcuts',
                label: text.shortcuts.sidebar,
                headerAction: {
                  id: 'manage',
                  label: text.shortcuts.manage,
                  icon: 'settings' as const,
                },
                footerAction: { id: 'new', label: text.shortcuts.create, icon: 'plus' as const },
                reorder: { label: text.shortcuts.reorder },
                items: shortcuts.items.map((shortcut) => ({
                  id: `shortcut:${shortcut.id}`,
                  label: shortcut.name,
                  href: href('/', { ...shortcut.search, shortcut: shortcut.id }),
                  current:
                    location.pathname === '/' &&
                    (location.search as { shortcut?: string }).shortcut === shortcut.id,
                  ...(listed === undefined
                    ? {}
                    : { count: shellCount(listPages(listed, shortcut.search).length, 'plain') }),
                  actions: [
                    {
                      id: 'edit',
                      label: text.shortcuts.editRow(shortcut.name),
                      icon: 'pencil' as const,
                    },
                    {
                      id: 'delete',
                      label: text.shortcuts.deleteRow(shortcut.name),
                      icon: 'trash-2' as const,
                    },
                  ],
                })),
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
  }, [
    answer,
    listed,
    named,
    actions,
    location.pathname,
    location.search,
    scope,
    shortcuts.store,
    shortcuts.items,
  ]);
  const select = (id: string) => {
    if (id === 'new-project' && actions) return (actions.newProject(), true);
    if (id === 'manage-projects' && actions) return (actions.manage(), true);
    if (id.startsWith('shortcut:')) {
      const item = shortcuts.items.find((shortcut) => shortcut.id === id.slice('shortcut:'.length));
      if (item) void router.navigate({ to: '/', search: { ...item.search, shortcut: item.id } });
      return true;
    }
    if (id === 'colab-settings') return (void router.navigate({ to: '/settings' }), true);
    if (id.startsWith('shell:')) {
      const pageId = id.slice('shell:'.length);
      void router.navigate({ to: '/pages/$pageId', params: { pageId } });
      return true;
    }
    if (id === 'all' && (location.search as { shortcut?: string }).shortcut) {
      writeScope(space, undefined);
      void router.navigate({ to: '/', search: {} });
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
  const menuAction = (event: ShellMenuActionEvent) => {
    if (event.groupId !== 'shortcuts') return;
    setShortcutError(undefined);
    if (event.actionId === 'manage') setShortcutDialog({ mode: 'manage' });
    else if (event.actionId === 'new') setShortcutDialog({ mode: 'create' });
    else if (event.itemId?.startsWith('shortcut:')) {
      const id = event.itemId.slice('shortcut:'.length);
      if (!shortcuts.items.some((item) => item.id === id)) return;
      if (event.actionId === 'edit') setShortcutDialog({ mode: 'edit', id });
      else if (event.actionId === 'delete') setDeleteShortcut(id);
    }
  };
  const menuReorder = (event: ShellMenuReorderEvent) => {
    if (event.groupId !== 'shortcuts' || !shortcuts.store || !event.itemId.startsWith('shortcut:'))
      return;
    try {
      shortcuts.store.reorder(
        event.itemId.slice('shortcut:'.length),
        event.beforeId?.slice('shortcut:'.length) ?? null,
      );
      setShortcutError(undefined);
    } catch {
      setShortcutError(text.shortcuts.saveFailed);
    }
  };
  const selected = shortcuts.items.find((item) => item.id === shortcutDialog?.id);
  const deleting = shortcuts.items.find((item) => item.id === deleteShortcut);
  const saved = (item: Shortcut) => {
    setShortcutDialog(undefined);
    if (
      location.pathname === '/' &&
      (location.search as { shortcut?: string }).shortcut === item.id
    )
      void router.navigate({
        to: '/',
        search: { ...item.search, shortcut: item.id },
        replace: true,
      });
  };
  const content = (
    <ProjectMenuActionsSlot value={setActions}>
      {children}
      {shortcutError && !deleting && <p role="alert">{shortcutError}</p>}
      {shortcutDialog && shortcuts.store && (shortcutDialog.mode !== 'edit' || selected) && (
        <ShortcutDialog
          key={`${space}:${shortcutDialog.mode}:${shortcutDialog.id ?? ''}`}
          mode={shortcutDialog.mode}
          initial={selected ?? (shortcutDialog.mode === 'manage' ? shortcuts.items[0] : undefined)}
          search={selected?.search ?? {}}
          store={shortcuts.store}
          items={shortcuts.items}
          pages={listed}
          projects={named ?? NO_PROJECTS}
          error={shortcuts.error}
          onClose={() => setShortcutDialog(undefined)}
          onSaved={saved}
        />
      )}
      {deleting && (
        <BrowserDialog
          open
          title={text.shortcuts.deleteTitle(deleting.name)}
          onDismiss={() => setDeleteShortcut(undefined)}
          actions={
            <>
              <BrowserAction
                type="button"
                variant="secondary"
                label={text.pageList.cancel}
                onActivate={() => setDeleteShortcut(undefined)}
              />
              <BrowserAction
                type="button"
                variant="destructive"
                label={text.pageList.remove}
                onActivate={() => {
                  try {
                    shortcuts.store?.delete(deleting.id);
                    setDeleteShortcut(undefined);
                    setShortcutError(undefined);
                  } catch {
                    setShortcutError(text.shortcuts.saveFailed);
                  }
                }}
              />
            </>
          }
        >
          <p>{text.shortcuts.deleteBody}</p>
          {shortcutError && <p role="alert">{shortcutError}</p>}
        </BrowserDialog>
      )}
    </ProjectMenuActionsSlot>
  );
  return hostSlot ? (
    <HostedShell
      slot={hostSlot}
      menu={view.menu}
      select={select}
      menuAction={menuAction}
      menuReorder={menuReorder}
    >
      <SharedTopBar value={hostSlot.topBarReady()}>{content}</SharedTopBar>
    </HostedShell>
  ) : (
    <OwnShell view={view} select={select} menuAction={menuAction} menuReorder={menuReorder}>
      {content}
    </OwnShell>
  );
}

/** Colab's own shell is the root of its common controls: Search opens the quick switcher the
 * route's host owns, and Remote shows the live-session count this page already reads, linking to the
 * rail's Remote until a count is confirmed. */
function OwnShell({
  view,
  select,
  menuAction,
  menuReorder,
  children,
}: {
  view: ShellView;
  select(id: string): boolean;
  menuAction(event: ShellMenuActionEvent): void;
  menuReorder(event: ShellMenuReorderEvent): void;
  children: ReactNode;
}) {
  const switcher = useQuickSwitcher();
  const owner = useRef(switcher);
  owner.current = switcher;
  const remote = useShellRemoteStatus(
    view.rail.main.find((item) => item.id === 'remote')?.href ?? '/remote/',
  );
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
    <BrowserShell
      view={view}
      select={select}
      menuAction={menuAction}
      menuReorder={menuReorder}
      commonControls={commonControls}
    >
      <SharedTopBar value>{children}</SharedTopBar>
    </BrowserShell>
  );
}
