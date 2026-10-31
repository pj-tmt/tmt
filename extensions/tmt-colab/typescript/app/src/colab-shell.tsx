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
  type ShellMenuItem,
  type ShellView,
} from '@tmt/browser-ui/shell';
import { scopeRows, writeScope } from './project-scope.js';
import { ShortcutMenuRequestSlot, type ShortcutMenuRequest } from './shortcut-menu-request.js';
import { shortcutCounts, type Shortcut } from './shortcuts.js';
import { useQuickSwitcher } from './quick-switcher-context.js';
import { useShellRemoteStatus } from './remote-header.js';
import { SharedTopBar } from './shared-top-bar.js';
import { onRememberedPages, rememberedPages } from './quick-switcher-source.js';
import { text } from './strings.js';
import type { PageSummary, PageTransport } from './transport.js';
import { useShortcuts } from './use-shortcuts.js';

/** The TMT app's slot when Colab runs as its `/colab/` route; the app owns the rail then. */
export const ShellHostSlot = createContext<ShellSlot | null>(null);
const NO_PAGES: readonly PageSummary[] = [];
const NO_PROJECTS: readonly string[] = [];

/** The Shortcuts group's rows: each saved shortcut, opening the list with its filters. */
export function shortcutRows(
  items: readonly Shortcut[],
  current: string | undefined,
  link: (item: Shortcut) => string,
  pages: readonly PageSummary[] | undefined,
  actions?: (item: Shortcut) => ShellMenuItem['actions'],
): ShellMenuItem[] {
  return items.map((item) => {
    const counts = shortcutCounts(pages, item.search);
    return {
      id: `shortcut:${item.id}`,
      icon: 'bookmark',
      label: item.name,
      href: link(item),
      current: item.id === current,
      total: counts.total,
      count: counts.attention ? shellCount(counts.attention) : undefined,
      actions: actions?.(item),
    };
  });
}

/**
 * Colab inside the one TMT web shell. The rail comes from Remote's composed descriptions; the
 * module menu is Colab's own fixed list: All pages, the Shortcuts group, Dashboards (the shell
 * pages, when there are any), then Colab settings. Exactly one row is selected: settings or a
 * dashboard on its route, else the shortcut the list shows (or the list a page was opened from),
 * else All pages.
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
  const shortcuts = useShortcuts(space);
  const manageable = transport.management !== undefined && shortcuts.store !== undefined;
  // The rail's Shortcuts commands open the list's own dialogs, opening the list first if needed.
  const [request, setRequest] = useState<ShortcutMenuRequest>();
  const shortcutRequest = useMemo(
    () => ({
      request,
      done: (id: number) => setRequest((held) => (held?.id === id ? undefined : held)),
    }),
    [request],
  );
  const [shortcutError, setShortcutError] = useState<string>();
  const pendingShortcutFocus = useRef<string | null | undefined>(undefined);
  const openShortcuts = (kind: ShortcutMenuRequest['kind'], shortcut?: string) => {
    setRequest({ kind, id: Date.now(), shortcut });
    if (router.state.location.pathname !== '/') void router.navigate({ to: '/' });
  };
  // The menu reads nothing itself: it shows the list the routes already read (the space home, a
  // link-opened page's chip read, the quick switcher), so it never adds a list request.
  const listed = useSyncExternalStore(onRememberedPages, () => rememberedPages(transport, space));
  useEffect(() => {
    if (hostSlot) return;
    void shellAddress().then(async (address) => {
      const read = address === undefined ? undefined : await loadShellAnswer(address);
      if (read) setAnswer(read);
    });
  }, [hostSlot]);

  const listSearch =
    location.pathname === '/' ? (location.search as { shortcut?: string }) : undefined;
  // A page selects the shortcut its list was showing; a link-opened page selects All pages.
  const lastShortcut = useRef<string | undefined>(undefined);
  if (listSearch) lastShortcut.current = listSearch.shortcut;
  const shortcutId = location.pathname.startsWith('/pages/')
    ? lastShortcut.current
    : listSearch?.shortcut;
  const href = (to: string, search?: Record<string, unknown>) =>
    router.history.createHref(router.buildLocation({ to, search }).publicHref);
  useEffect(() => {
    const target = pendingShortcutFocus.current;
    if (target === undefined) return;
    pendingShortcutFocus.current = undefined;
    const frame = requestAnimationFrame(() => {
      const controls = target
        ? document.querySelectorAll<HTMLElement>('.tmt-ui-rail-sub a[data-id]')
        : document.querySelectorAll<HTMLElement>(
            '.tmt-ui-rail-sub-group .tmt-ui-rail-group-action',
          );
      const next = [...controls].find(
        (node) =>
          node.getClientRects().length > 0 &&
          (target === null || node.dataset.id === `shortcut:${target}`),
      );
      const fallback = [...document.querySelectorAll<HTMLElement>('.tmt-ui-shell-open')].find(
        (node) => node.getClientRects().length > 0,
      );
      (next ?? fallback)?.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(frame);
  }, [shortcuts.items]);
  const view = useMemo(() => {
    const shown = (listed ?? NO_PAGES).filter((page) => !page.archived);
    const settings = location.pathname === '/settings';
    // A dashboard selects its own row, ahead of the shortcut its list showed.
    const shells = shown
      .filter((page) => page.pageType === 'shell')
      .map((page) => ({
        id: `shell:${page.id}`,
        label: page.title,
        href: router.history.createHref(
          router.buildLocation({ to: '/pages/$pageId', params: { pageId: page.id } }).publicHref,
        ),
        current: location.pathname === `/pages/${page.id}`,
      }));
    const dashboard = shells.some((row) => row.current);
    const rows = shortcutRows(
      shortcuts.items,
      settings || dashboard ? undefined : shortcutId,
      (item) => href('/', { ...item.search, shortcut: item.id }),
      listed,
      manageable
        ? (item) => [
            {
              label: text.shortcuts.editRow(item.name),
              icon: 'pencil',
              onActivate: () => openShortcuts('edit', item.id),
            },
            {
              label: text.shortcuts.deleteRow(item.name),
              icon: 'trash-2',
              onActivate: () => {
                try {
                  const latest = shortcuts.store!.read().items;
                  const index = latest.findIndex((candidate) => candidate.id === item.id);
                  if (index < 0) return;
                  const next = latest[index + 1] ?? latest[index - 1];
                  pendingShortcutFocus.current = next?.id ?? null;
                  shortcuts.store!.delete(item.id);
                  setShortcutError(undefined);
                  if (shortcutId === item.id) void router.navigate({ to: '/', search: {} });
                } catch {
                  pendingShortcutFocus.current = undefined;
                  setShortcutError(text.shortcuts.operationFailed);
                }
              },
            },
          ]
        : undefined,
    );
    const [all] = scopeRows(shown, NO_PROJECTS);
    const menu: ShellMenu = {
      title: text.product,
      groups: [
        {
          items: [
            {
              id: 'all',
              icon: 'list',
              label: text.pageList.allPages,
              href: href('/'),
              total: all!.count,
              count: all!.waiting ? shellCount(all!.waiting) : undefined,
              current: !settings && !dashboard && !rows.some((row) => row.current),
            },
          ],
        },
        {
          label: text.shortcuts.group,
          ...(manageable
            ? {
                action: {
                  label: text.shortcuts.manageAction,
                  onActivate: () => openShortcuts('manage'),
                },
              }
            : {}),
          items: rows,
          ...(manageable
            ? {
                reorder: (from: string, to: string) => {
                  const prefix = 'shortcut:';
                  if (!from.startsWith(prefix) || !to.startsWith(prefix)) return;
                  try {
                    shortcuts.store!.moveTo(from.slice(prefix.length), to.slice(prefix.length));
                    setShortcutError(undefined);
                  } catch {
                    setShortcutError(text.shortcuts.operationFailed);
                  }
                },
              }
            : {}),
        },
        ...(shells.length ? [{ label: text.colabSettings.shells, items: shells }] : []),
      ],
      settings: {
        id: 'colab-settings',
        label: text.colabSettings.title,
        href: href('/settings'),
        current: settings,
      },
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
  }, [answer, listed, shortcuts.items, shortcuts.store, manageable, location.pathname, shortcutId]);
  const select = (id: string) => {
    if (id === 'colab-settings') return (void router.navigate({ to: '/settings' }), true);
    if (id.startsWith('shell:')) {
      const pageId = id.slice('shell:'.length);
      void router.navigate({ to: '/pages/$pageId', params: { pageId } });
      return true;
    }
    const shortcut = id.startsWith('shortcut:')
      ? shortcuts.items.find((item) => `shortcut:${item.id}` === id)
      : undefined;
    if (id === 'colab' || id === 'all' || shortcut) {
      // A fixed destination opens the list fresh; the remembered project would otherwise narrow it.
      if (id !== 'colab') writeScope(space, undefined);
      void router.navigate({
        to: '/',
        search: shortcut ? { ...shortcut.search, shortcut: shortcut.id } : {},
      });
      return true;
    }
    return false;
  };
  const content = (
    <ShortcutMenuRequestSlot value={shortcutRequest}>
      {shortcutError && <p role="alert">{shortcutError}</p>}
      {children}
    </ShortcutMenuRequestSlot>
  );
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
 * rail's Remote until a count is confirmed. */
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
    <BrowserShell view={view} select={select} commonControls={commonControls}>
      <SharedTopBar value>{children}</SharedTopBar>
    </BrowserShell>
  );
}
