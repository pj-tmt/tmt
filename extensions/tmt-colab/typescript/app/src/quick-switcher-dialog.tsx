import { BrowserDialog } from '@tmt/browser-ui/react';
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { Listbox } from './components/listbox.js';
import { labelCounts, authorCounts } from './labels.js';
import {
  PageListSearch,
  editFromSearch,
  searchFromEdit,
  type SearchEdit,
} from './page-list-search.js';
import { pageTitle } from './page-index.js';
import { rankPages } from './quick-switcher.js';
import { useSwitcherSource } from './quick-switcher-source.js';
import { QuickSwitcherContext, useQuickSwitcher } from './quick-switcher-context.js';
import { admitsSwitcherShortcut } from './quick-switcher-key.js';
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

/** Routing and page data stay Colab-owned; the leaf only contains and dismisses the surface. */
export function QuickSwitcherHost({
  transport,
  space,
  pageId,
  routeKey,
  allowed,
  navigate,
  children,
}: {
  transport: PageTransport;
  space?: string;
  pageId?: string;
  routeKey: string;
  allowed: boolean;
  navigate(pageId: string): Promise<unknown>;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const control = useRef<HTMLButtonElement>(null);
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
  }, []);
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (admitsSwitcherShortcut(event) && activate()) event.preventDefault();
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [activate]);
  const presentation = useMemo(
    () => (allowed ? { open, activate, control } : null),
    [allowed, open, activate],
  );
  const source = useSwitcherSource(open, transport, space);
  const filter = searchFromEdit(edit);
  const results =
    source.state === 'ready'
      ? rankPages(
          source.pages,
          { labels: filter.labels, authors: filter.authors, text: filter.q },
          source.opened,
          { exclude: pageId },
        )
      : [];
  const choose = (id: string) => {
    // A refresh may remove an option between pointer down and activation.
    if (!latest.current.open || !results.some((row) => row.page.id === id)) return;
    restoring.current = false;
    setOpen(false);
    void latest.current
      .navigate(id)
      .then(() => {
        if (!mounted.current || latest.current.pageId !== id) return;
        const title = document.querySelector<HTMLElement>('.tmt-ui-header h1');
        if (title) {
          title.tabIndex = -1;
          title.focus({ preventScroll: true });
        }
      })
      .catch(() => {
        // The router owns its load-error surface; no second navigation or retry is admitted.
        if (mounted.current && !latest.current.open)
          control.current?.focus({ preventScroll: true });
      });
  };
  const hasFilter = !!(filter.q || filter.labels.length || filter.authors.length);
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
  return (
    <QuickSwitcherContext value={presentation}>
      {children}
      <BrowserDialog
        title={t.title}
        open={open}
        className="quick-switcher-dialog"
        data-quick-switcher
        initialFocus={() =>
          inputHost.current?.querySelector<HTMLElement>('[role="combobox"]') ?? null
        }
        focusFallback={() => control.current}
        restoreFocus={restoring.current}
        onDismiss={() => setOpen(false)}
      >
        <div ref={inputHost}>
          <Listbox
            label={t.results}
            presentation="results"
            options={results.map(({ page }) => ({ value: page.id, label: pageTitle(page) }))}
            value=""
            onChange={choose}
            renderOption={({ value }) => {
              const row = results.find(({ page }) => page.id === value)!.page;
              return (
                <>
                  <span className="quick-switcher-title">{pageTitle(row)}</span>
                  <span className="quick-switcher-meta">
                    {row.summary?.author && <span>{row.summary.author}</span>}
                    {(row.labels ?? []).map((label) => (
                      <span key={label}>{label}</span>
                    ))}
                    {row.summary?.waiting && (
                      <span className="page-waiting">
                        <span aria-hidden="true">◆</span> {text.pageList.waitingForYou}
                      </span>
                    )}
                  </span>
                </>
              );
            }}
            inputTrigger={{
              open: results.length > 0,
              onOpenChange() {},
              render: (trigger) => (
                <PageListSearch
                  edit={edit}
                  onChange={setEdit}
                  resetKey={resetKey}
                  labels={source.state === 'ready' ? labelCounts(source.pages) : []}
                  authors={source.state === 'ready' ? authorCounts(source.pages) : []}
                  onApply={setEdit}
                  onClear={() => setOpen(false)}
                  placeholder={t.placeholder}
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
              ),
            }}
          />
          {!results.length && (
            <p className="quick-switcher-message" role="status">
              {message}
            </p>
          )}
        </div>
      </BrowserDialog>
    </QuickSwitcherContext>
  );
}
