import { focusPageHeading } from './colab-header.js';
import { BrowserDialog, BrowserMenu } from '@tmt/browser-ui/react';
import type { SearchGroup, SearchRegistry } from '@tmt/browser-ui/search';
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import {
  ArrowRight,
  Box,
  ChartColumn,
  Clock,
  FileText,
  Folder,
  House,
  MonitorSmartphone,
  Search,
  Settings,
  Terminal,
  Users,
  type LucideIcon,
} from 'lucide-react';
import { Listbox } from './components/listbox.js';
import { projectCounts, authorCounts } from './projects.js';
import {
  PageListSearch,
  editFromSearch,
  searchFromEdit,
  toggleProject,
  type SearchEdit,
} from './page-list-search.js';
import { pageTitle } from './page-index.js';
import { rankPages } from './quick-switcher.js';
import { composeEntries, scopeNames, type SwitcherEntry } from './quick-switcher-groups.js';
import { useSwitcherSource } from './quick-switcher-source.js';
import { QuickSwitcherContext, useQuickSwitcher } from './quick-switcher-context.js';
import { admitsSwitcherShortcut, usesCommandKey } from './quick-switcher-key.js';
import { text } from './strings.js';
import type { PageTransport } from './transport.js';
import { useNotificationModalPresence } from './reply-notification-context.js';
import './quick-switcher-dialog.css';

const t = text.quickSwitcher;

/** Report the shell's actual modal state into the existing page-local reply queue. */
export function QuickSwitcherModalPresence() {
  const owner = useQuickSwitcher();
  const report = useNotificationModalPresence();
  useLayoutEffect(() => {
    report(owner?.open === true);
  }, [report, owner?.open]);
  return null;
}

/** The rail glyph names a provider item may carry, as the icons this dialog draws. */
const ITEM_ICONS: Record<string, LucideIcon> = {
  house: House,
  'file-text': FileText,
  'monitor-smartphone': MonitorSmartphone,
  users: Users,
  'chart-column': ChartColumn,
  settings: Settings,
  folder: Folder,
  box: Box,
  terminal: Terminal,
  clock: Clock,
};
const itemIcon = (name: string | undefined): LucideIcon => ITEM_ICONS[name ?? ''] ?? ArrowRight;

/**
 * What turns the page switcher into the global search: the registry whose groups list beside the
 * pages, and the host's own navigation for everything that is not a Colab page.
 */
export interface SwitcherSearch {
  registry: SearchRegistry;
  /** Opens a route of the TMT app that is not a Colab page: a module, a section, a provider item. */
  go(href: string): Promise<unknown>;
  /** The page list scoped to one project; without it no project is offered. */
  projectHref?(name: string): string;
  /** The field's placeholder; without it the placeholder names the groups that are ready. */
  placeholder?: string;
}
const NO_GROUPS: readonly SearchGroup[] = [];
const noSubscription = () => () => {};

/** Routing and page data stay Colab-owned; the leaf only contains and dismisses the surface. */
export function QuickSwitcherHost({
  transport,
  space,
  pageId,
  routeKey,
  allowed,
  navigate,
  pageHref,
  search,
  shortcut = true,
  onReady,
  children,
}: {
  transport: PageTransport;
  space?: string;
  pageId?: string;
  routeKey: string;
  allowed: boolean;
  navigate(pageId: string): Promise<unknown>;
  pageHref(pageId: string): string;
  search?: SwitcherSearch;
  /** False when the host's own bar owns the one Command/Ctrl+K listener. */
  shortcut?: boolean;
  /** Hands the opener to a host that opens the switcher from its own control. */
  onReady?(open: (opener: HTMLButtonElement) => boolean): void;
  children?: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const control = useRef<HTMLButtonElement | null>(null);
  const inputHost = useRef<HTMLDivElement>(null);
  const [edit, setEdit] = useState<SearchEdit>(() => editFromSearch([], ''));
  const [resetKey, setResetKey] = useState(0);
  const restoring = useRef(true);
  const mounted = useRef(true);
  const latest = useRef({ open, allowed, navigate, pageId });
  latest.current = { open, allowed, navigate, pageId };
  useLayoutEffect(() => {
    setOpen(false);
  }, [routeKey, transport, space]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const activate = useCallback(() => {
    if (!latest.current.allowed || latest.current.open) return false;
    if (
      [...document.querySelectorAll<HTMLElement>('dialog[open],[role="menu"]')].some(
        (node) => node.getClientRects().length > 0,
      )
    )
      return false;
    latest.current.open = true;
    restoring.current = true;
    setEdit(editFromSearch([], ''));
    setResetKey((value) => value + 1);
    setOpen(true);
    return true;
  }, [setEdit]);
  useEffect(() => {
    if (!shortcut) return;
    const key = (event: KeyboardEvent) => {
      if (admitsSwitcherShortcut(event) && activate()) event.preventDefault();
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [activate, shortcut]);
  useEffect(() => {
    onReady?.((opener) => {
      control.current = opener;
      return activate();
    });
  }, [onReady, activate]);
  const presentation = useMemo(
    () => (allowed ? { open, activate, control } : null),
    [allowed, open, activate],
  );
  const source = useSwitcherSource(open, transport, space);
  const providers = useSyncExternalStore(
    search?.registry.subscribe ?? noSubscription,
    () => search?.registry.groups() ?? NO_GROUPS,
  );
  const filter = searchFromEdit(edit);
  const results =
    source.state === 'ready'
      ? rankPages(
          source.pages,
          { projects: filter.projects, authors: filter.authors, text: filter.q },
          source.opened,
          { exclude: pageId },
        )
      : [];
  const entries = composeEntries({
    ...(search ? { providers } : {}),
    pages: results,
    projects: search?.projectHref && source.state === 'ready' ? projectCounts(source.pages) : [],
    text: filter.q,
    colabLabel: t.colab,
  });
  const hrefOf = (entry: SwitcherEntry): string | undefined =>
    entry.kind === 'page'
      ? pageHref(entry.result.page.id)
      : entry.kind === 'project'
        ? search?.projectHref?.(entry.name)
        : entry.item.href;
  const choose = (value: string) => {
    // A refresh or a provider may remove an option between pointer down and activation.
    const entry = entries.find((row) => row.value === value);
    if (!latest.current.open || !entry) return;
    if (entry.kind !== 'page') {
      // A provider item is chosen as the latest admitted snapshot has it, never as it was drawn.
      const href =
        entry.kind === 'item'
          ? search?.registry.find(entry.groupKey, entry.item.key)?.href
          : hrefOf(entry);
      if (!href || !search) return;
      restoring.current = true;
      setOpen(false);
      void search.go(href).catch(() => {
        // The host owns its own load-error surface; no second navigation or retry is admitted.
        if (mounted.current && !latest.current.open)
          control.current?.focus({ preventScroll: true });
      });
      return;
    }
    const id = entry.result.page.id;
    restoring.current = false;
    setOpen(false);
    void latest.current
      .navigate(id)
      .then(() => {
        if (!mounted.current || latest.current.pageId !== id) return;
        focusPageHeading();
      })
      .catch(() => {
        // The router owns its load-error surface; no second navigation or retry is admitted.
        if (mounted.current && !latest.current.open)
          control.current?.focus({ preventScroll: true });
      });
  };
  const hasFilter = !!(filter.q || filter.projects.length || filter.authors.length);
  const message =
    source.state === 'failed'
      ? t.failed
      : source.state !== 'ready'
        ? t.loading
        : hasFilter
          ? t.noMatch
          : source.pages.length === 0
            ? t.empty
            : pageId &&
                source.pages.filter((row) => !row.archived).every((row) => row.id === pageId)
              ? t.noOtherPages
              : t.noMatch;
  // The global list shows what it has while the pages load or fail, and says so beneath it.
  const status = entries.length ? (search && source.state !== 'ready' ? message : '') : message;
  const optionOf = (entry: SwitcherEntry) => ({
    value: entry.value,
    label:
      entry.kind === 'page'
        ? pageTitle(entry.result.page)
        : entry.kind === 'project'
          ? entry.name
          : entry.item.label,
    ...(hrefOf(entry) ? { href: hrefOf(entry)! } : {}),
    ...(entry.group ? { group: entry.group } : {}),
  });
  return (
    <QuickSwitcherContext value={presentation}>
      {children}
      <BrowserDialog
        title={search ? t.titleGlobal : t.title}
        open={open}
        className="quick-switcher-dialog"
        data-quick-switcher
        data-global={search ? '' : undefined}
        initialFocus={() =>
          inputHost.current?.querySelector<HTMLElement>('[role="combobox"]') ?? null
        }
        focusFallback={() => control.current}
        restoreFocus={restoring.current}
        onDismiss={() => setOpen(false)}
        actions={
          <div className="quick-switcher-shortcuts">
            <span>
              <kbd>↑ ↓</kbd> {t.navigate}
            </span>
            <span>
              <kbd>↵</kbd> {t.open}
            </span>
            <span>
              <kbd>esc</kbd> {t.close}
            </span>
          </div>
        }
      >
        <div ref={inputHost}>
          <Listbox
            label={search ? t.resultsGlobal : t.results}
            presentation="results"
            options={entries.map(optionOf)}
            value=""
            onChange={choose}
            renderOption={({ value }) => {
              const entry = entries.find((row) => row.value === value)!;
              if (entry.kind !== 'page') {
                const Icon = entry.kind === 'project' ? Folder : itemIcon(entry.item.icon);
                const detail =
                  entry.kind === 'project' ? t.projectPages(entry.pages) : entry.item.detail;
                return (
                  <>
                    <Icon className="quick-switcher-file" aria-hidden="true" />
                    <span className="quick-switcher-copy">
                      <span className="quick-switcher-title">
                        {entry.kind === 'project' ? entry.name : entry.item.label}
                      </span>
                      {detail && <span className="quick-switcher-meta">{detail}</span>}
                    </span>
                  </>
                );
              }
              const row = entry.result.page;
              return (
                <>
                  <FileText className="quick-switcher-file" aria-hidden="true" />
                  <span className="quick-switcher-copy">
                    <span className="quick-switcher-title">{pageTitle(row)}</span>
                    <span className="quick-switcher-meta">
                      {[
                        ...(row.projects ?? []),
                        ...(row.summary?.author ? [row.summary.author] : []),
                      ].join(' · ')}
                    </span>
                  </span>
                  {row.summary?.waiting && (
                    <span className="page-waiting quick-switcher-waiting">
                      <span aria-hidden="true">◆</span> {text.pageList.waitingForYou}
                    </span>
                  )}
                </>
              );
            }}
            inputTrigger={{
              open: entries.length > 0,
              onOpenChange() {},
              render: (trigger) => (
                <>
                  <div className="quick-switcher-search">
                    <Search className="quick-switcher-search-mark" aria-hidden="true" />
                    <PageListSearch
                      edit={edit}
                      onChange={setEdit}
                      resetKey={resetKey}
                      projects={source.state === 'ready' ? projectCounts(source.pages) : []}
                      authors={source.state === 'ready' ? authorCounts(source.pages) : []}
                      onApply={setEdit}
                      onClear={() => setOpen(false)}
                      placeholder={
                        search
                          ? (search.placeholder ??
                            t.placeholderGlobal(scopeNames(providers, t.pagesNoun)))
                          : t.placeholder
                      }
                      label={search ? t.titleGlobal : undefined}
                      resultTrigger={trigger}
                      onUnhandledKeyDown={(event) => {
                        if (event.key === 'Escape') {
                          event.preventDefault();
                          event.stopPropagation();
                          if (
                            event.repeat ||
                            event.altKey ||
                            event.metaKey ||
                            event.ctrlKey ||
                            event.shiftKey
                          )
                            return;
                          setOpen(false);
                        } else trigger.onKeyDown?.(event);
                      }}
                    />
                  </div>
                  <div className="quick-switcher-filter">
                    <Folder aria-hidden="true" />
                    {search && <span className="quick-switcher-scope">{t.colab}:</span>}
                    <BrowserMenu
                      label={t.projectFilter}
                      triggerText={
                        filter.projects.length ? filter.projects.join(', ') : t.allProjects
                      }
                      items={[
                        {
                          key: '',
                          label: t.allProjects,
                          role: 'menuitemradio',
                          checked: filter.projects.length === 0,
                        },
                        ...(source.state === 'ready' ? projectCounts(source.pages) : []).map(
                          ({ name }) => ({
                            key: name,
                            label: name,
                            role: 'menuitemcheckbox' as const,
                            checked: filter.projects.includes(name),
                          }),
                        ),
                      ]}
                      onSelect={(name, event) => {
                        if (!event.isTrusted) return;
                        setEdit(
                          name
                            ? toggleProject(edit, name)
                            : editFromSearch([], filter.q, filter.authors),
                        );
                      }}
                    />
                    <kbd className="quick-switcher-shortcut">
                      {usesCommandKey() ? '⌘K' : 'Ctrl K'}
                    </kbd>
                  </div>
                  <div className="quick-switcher-rule" />
                </>
              ),
            }}
          />
          {status && (
            <p className="quick-switcher-message" role="status">
              {status}
            </p>
          )}
        </div>
      </BrowserDialog>
    </QuickSwitcherContext>
  );
}
