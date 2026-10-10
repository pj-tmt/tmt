import { publicEntry } from './entry.js';
import type { ComposerEdit } from './components/message-composer-edit.js';
import {
  BrowserAction,
  BrowserCommand,
  BrowserIconAction,
  BrowserList,
  BrowserListRow,
  BrowserToggle,
} from '@tmt/browser-ui/react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { shortPageId, validPagePrefix } from './short-links.js';
import { ArrowUpRight, LoaderCircle, Moon, Sun } from 'lucide-react';
import { PageHeaderActions, type PageHeaderAction } from './page-header-actions.js';
import { ColabHeader } from './colab-header.js';
import { PageAttribution } from './page-attribution.js';
import { NoticeCard } from './notice-card.js';
import { RetentionHint } from './retention-hint.js';
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import {
  createHashHistory,
  createBrowserHistory,
  createRootRouteWithContext,
  createRoute,
  createRouter,
  Link,
  redirect,
  Outlet,
  useRouter,
  useRouterState,
} from '@tanstack/react-router';
import type { PageSummary, PageView, PageTransport } from './transport.js';
import { orderPages, pageTitle, pageUpdate } from './page-index.js';
import { AnnotationInput } from './annotation-input.js';
import { useProposalLayer } from './proposal-layer.js';
import { ReplyAnnouncer } from './reply-announcer.js';
import { ThreadPanel, ThreadWindow } from './thread-panel.js';
import { presentationOf } from './thread-status-presentation.js';
import { isStatusThread, openThreadCount } from './thread-status-view.js';
import type { QuoteSelector, DiscussionRef } from './thread-records.js';
import { ShareDialog } from './share-dialog.js';
import { mountRenderer, MAX_RENDER_SOURCE_BYTES } from './renderer.js';
import type { RenderState, SelectionRect } from './renderer.js';
import { text } from './strings.js';
import { getTheme, setThemeChoice, subscribeTheme } from './theme.js';
import { terminalFailure } from './terminal-failure.js';
import { buildWatch } from './app-build.js';
import { SessionEvictedError } from './ask-remote.js';
import { RecoveryRequiredError } from './session-recovery.js';
import { ExportPanel } from './export-panel.js';
import { PageDrawer } from './page-drawer.js';
import { AgentStatusPanel } from './agent-status-panel.js';
import { ChatPanel } from './chat-panel.js';
import { FilesPanel } from './files-panel.js';
import { DraftSession } from './draft-store.js';
import { CHAT_DRAFT, SavedDrafts, savedDrafts } from './saved-drafts.js';
import { isChatThread } from './thread-records.js';
import { SaveOutcomeUnknown, saveMessage } from './save.js';
import { SaveNotice, type SaveProblem } from './save-notice.js';

function SelectionAnnotation({
  host,
  rectangle,
  inset,
  noticeVisible,
  open,
  children,
}: {
  host: HTMLDivElement | null;
  rectangle: SelectionRect | null;
  inset: number;
  noticeVisible: boolean;
  open(): void;
  children?: React.ReactNode;
}) {
  const expanded = children !== undefined;
  const element = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{
    left: number;
    top?: number;
    bottom?: number;
    maxHeight: number;
  } | null>(null);
  useEffect(() => {
    const notice = noticeVisible
      ? host?.parentElement?.querySelector<HTMLElement>('.tmt-ui-notice')
      : null;
    const place = () => {
      const frame = host?.querySelector('iframe')?.getBoundingClientRect();
      const width = expanded ? Math.min(380, innerWidth - 24) : 100;
      const placementHeight = expanded ? Math.min(480, innerHeight - inset - 16) : 38;
      if (!frame || !rectangle) {
        // Keep the failure notice readable while retaining the mounted draft.
        const belowNotice = notice?.getBoundingClientRect().bottom;
        setPosition((previous) => {
          if (!expanded || !previous) return null;
          const top =
            belowNotice !== undefined
              ? Math.max(inset + 4, belowNotice + 8)
              : Math.max(
                  inset + 4,
                  Math.min(
                    innerHeight - placementHeight - 8,
                    element.current!.getBoundingClientRect().top,
                  ),
                );
          return {
            left: Math.max(8, Math.min(innerWidth - width - 8, previous.left)),
            top,
            maxHeight: Math.max(0, innerHeight - top - 8),
          };
        });
        return;
      }
      const selectionTop = frame.top + Math.max(0, Math.min(frame.height, rectangle.y)),
        selectionBottom =
          frame.top + Math.max(0, Math.min(frame.height, rectangle.y + rectangle.height)),
        left = frame.left + Math.max(0, Math.min(frame.width, rectangle.x)),
        right = frame.left + Math.max(0, Math.min(frame.width, rectangle.x + rectangle.width));
      if (!expanded && (selectionBottom < inset || selectionTop > innerHeight)) {
        setPosition(null);
        return;
      }
      // A completed selection can end outside the viewport. Its open draft still
      // needs bounded history and a visible composer, including when opened by C.
      const clampAnchor = (value: number) => Math.max(inset + 4, Math.min(innerHeight - 8, value));
      const top = expanded ? clampAnchor(selectionTop) : selectionTop;
      const bottom = expanded ? clampAnchor(selectionBottom) : selectionBottom;
      const beside = !expanded && right + width + 8 <= Math.min(frame.right, innerWidth - 8);
      const below = bottom + placementHeight + 8 <= innerHeight - 8;
      // Choose the edge using stable placement clearance, not the changing content
      // height. CSS fits the shell; first Send grows away from the same anchor edge.
      const above = expanded && !below && top - placementHeight - 6 >= inset + 4;
      const windowTop = Math.max(
        inset + 4,
        Math.min(
          innerHeight - placementHeight - 8,
          beside ? top : below ? bottom + 6 : top - placementHeight - 6,
        ),
      );
      setPosition({
        left: Math.max(
          8,
          Math.min(innerWidth - width - 8, frame.right - width - 8, beside ? right + 8 : left),
        ),
        ...(above
          ? { bottom: innerHeight - top + 6, maxHeight: top - 6 - inset - 4 }
          : { top: windowTop, maxHeight: Math.max(0, innerHeight - windowTop - 8) }),
      });
    };
    place();
    const observer = new ResizeObserver(place);
    if (element.current) observer.observe(element.current);
    if (notice) observer.observe(notice);
    window.addEventListener('scroll', place, { passive: true });
    window.addEventListener('resize', place);
    return () => {
      observer.disconnect();
      window.removeEventListener('scroll', place);
      window.removeEventListener('resize', place);
    };
  }, [host, rectangle, inset, expanded, noticeVisible, children]);
  return (
    <div
      ref={element}
      className={children ? 'annotation-popover' : 'selection-control'}
      style={{
        ...position,
        visibility: position ? 'visible' : 'hidden',
      }}
    >
      {children ?? (
        <button
          className={`selection-ask ${ui.action}`}
          data-testid="selection-ask"
          aria-keyshortcuts="c Alt+Enter"
          onPointerDown={(event) => event.preventDefault()}
          onClick={(event) => {
            if (event.isTrusted) open();
          }}
        >
          Annotate
        </button>
      )}
    </div>
  );
}
const managementChanged = 'Management changed. Reopen the page to load its latest state.';

/** Style the command in the same complete sentence used by live errors. */
function ReconnectFailure() {
  const [before, after] = text.reconnectFailed.split(text.reconnectCommand);
  return (
    <>
      {before}
      <code className="tmt-ui-code">{text.reconnectCommand}</code>
      {after}
    </>
  );
}

/** A terminal refusal as a sentence; its raw code stays as a small reference. */
function TerminalFailure({ error }: { error: Error }) {
  if (error.message === managementChanged) return <p>{managementChanged}</p>;
  const { sentence, reference } = terminalFailure(error);
  return (
    <>
      <p>{sentence === text.reconnectFailed ? <ReconnectFailure /> : sentence}</p>
      {reference && (
        <p>
          <small className="failure-reference">
            {text.failureCodeLabel}{' '}
            <code className="tmt-ui-code" data-failure-reference>
              {reference}
            </code>
          </small>
        </p>
      )}
    </>
  );
}

const root = createRootRouteWithContext<{
  transport: PageTransport;
  rememberPages?: (pages: readonly { pageId: string; deleted: boolean }[]) => void;
}>()({
  component: Shell,
  errorComponent: ({ error }) => (
    <>
      <AppHeader title={text.error} />
      <main>
        <NoticeCard
          state="blocked"
          eyebrow={text.product}
          title={text.error}
          actions={
            <Link className={ui.action} data-variant="text" to="/">
              {text.retry}
            </Link>
          }
        >
          <p>
            {error instanceof Error ? (
              error.message === text.reconnectFailed ? (
                <ReconnectFailure />
              ) : (
                error.message
              )
            ) : (
              text.blocked
            )}
          </p>
        </NoticeCard>
      </main>
    </>
  ),
  notFoundComponent: () => (
    <>
      <NoticeCard
        state="blocked"
        eyebrow={text.product}
        title={text.error}
        actions={
          <Link className={ui.action} data-variant="text" to="/">
            {text.retry}
          </Link>
        }
      />
    </>
  ),
});
const home = createRoute({
  getParentRoute: () => root,
  path: '/',
  loader: async ({ context }) => {
    const home = await context.transport.spaceHome();
    context.rememberPages?.(
      home.pageIds ?? home.pages.map((page) => ({ pageId: page.id, deleted: false })),
    );
    return home;
  },
  component: Home,
});
const page = createRoute({
  getParentRoute: () => root,
  path: '/pages/$pageId',
  loader: ({ context, params, abortController }) =>
    context.transport.page(params.pageId, abortController.signal),
  component: Page,
});

const shortPage = createRoute({
  getParentRoute: () => root,
  path: '/short/$prefix',
  loader: async ({ context, params }) => {
    if (!validPagePrefix(params.prefix)) throw new Error('Page unavailable');
    const home = await context.transport.spaceHome();
    const known = home.pageIds ?? home.pages.map((page) => ({ pageId: page.id, deleted: false }));
    context.rememberPages?.(known);
    const pages = known
      .filter((page) => page.pageId.startsWith(params.prefix))
      .map((row) => ({
        id: row.pageId,
        deleted: row.deleted,
        title: row.deleted
          ? ''
          : (home.pages.find((page) => page.id === row.pageId)?.title ?? text.unknownPageTitle),
        archived: row.deleted ? false : home.pages.find((page) => page.id === row.pageId)?.archived,
      }));
    if (pages.length === 0) throw new Error('Page unavailable');
    if (pages.length === 1 && !pages[0].deleted)
      throw redirect({ to: '/pages/$pageId', params: { pageId: pages[0].id }, replace: true });
    return { pages, prefix: params.prefix };
  },
  component: ShortPageChoice,
});
function ShortPageChoice() {
  const { pages, prefix } = shortPage.useLoaderData();
  const deleted = pages.length === 1 && pages[0].deleted;
  return (
    <NoticeCard
      state={deleted ? 'blocked' : 'waiting'}
      eyebrow={text.pages}
      title={deleted ? 'This page was deleted' : 'Choose a page'}
      testId="short-page-choice"
      actions={
        deleted ? (
          <Link className={ui.action} data-variant="text" to="/">
            Back to pages
          </Link>
        ) : undefined
      }
    >
      {deleted ? (
        <p>Its owner deleted it. Ask them for a new link if you still need it.</p>
      ) : (
        <p>More than one page matches {prefix}. Choose the page you want to open.</p>
      )}
      <ul className="short-page-options">
        {pages.map((page) => (
          <li key={page.id}>
            {page.deleted ? (
              <span aria-disabled="true">
                Deleted page
                <br />
                <code className="tmt-ui-code">{page.id}</code>
              </span>
            ) : (
              <Link to="/pages/$pageId" params={{ pageId: page.id }}>
                <span>
                  <span className="short-page-title">
                    {page.title}
                    {page.archived ? ' · Archived' : ''}
                  </span>
                  <br />
                  <code className="tmt-ui-code">{page.id}</code>
                </span>
                <ArrowUpRight aria-hidden />
              </Link>
            )}
          </li>
        ))}
      </ul>
    </NoticeCard>
  );
}

const blocked = createRoute({
  getParentRoute: () => root,
  path: '/blocked',
  loader: () => {
    throw new Error(text.pinMismatch);
  },
});

export function AppHeader({
  linked = true,
  title = text.pages,
}: {
  linked?: boolean;
  title?: string;
}) {
  return (
    <ColabHeader
      title={title}
      home={
        linked
          ? (brand) => (
              <Link to="/" aria-label={text.home}>
                {brand}
              </Link>
            )
          : undefined
      }
      actions={<ThemeButton />}
    />
  );
}
function ThemeButton() {
  const dark = useSyncExternalStore(subscribeTheme, getTheme) === 'dark';
  return (
    <span className="theme">
      <BrowserIconAction
        type="button"
        variant="text"
        label={text.theme}
        icon={dark ? <Moon /> : <Sun />}
        onActivate={() => {
          setThemeChoice(getTheme() === 'dark' ? 'light' : 'dark');
        }}
      />
    </span>
  );
}
function Shell() {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const isPage = pathname.startsWith('/pages/');
  return (
    <>
      {!isPage && (
        <AppHeader
          title={
            pathname.startsWith('/short/')
              ? 'Open a page'
              : pathname === '/'
                ? text.pages
                : text.error
          }
        />
      )}
      <main className={isPage ? 'page-main' : undefined}>
        <Outlet />
      </main>
      {!isPage && (
        <footer>{location.pathname.startsWith('/r/') ? text.mountedNote : text.adapter}</footer>
      )}
    </>
  );
}
function ManagePage({
  pageId,
  title,
  open,
  close,
  changed,
}: {
  pageId: string;
  title: string;
  open: boolean;
  close(): void;
  changed?(): void;
}) {
  const port = root.useRouteContext().transport.management;
  const router = useRouter();
  const touched = useRef(false);
  return open && port
    ? createPortal(
        <ShareDialog
          port={port}
          pageId={pageId}
          title={title}
          committed={() => {
            touched.current = true;
            changed?.();
          }}
          close={() => {
            close();
            if (touched.current) {
              touched.current = false;
              void router.invalidate();
            }
          }}
        />,
        document.body,
      )
    : null;
}
function ManageButton({ pageId, title }: { pageId: string; title: string }) {
  const port = root.useRouteContext().transport.management;
  const [open, setOpen] = useState(false);
  if (!port) return null;
  return (
    <>
      <BrowserAction
        type="button"
        variant="text"
        label={text.managePage}
        onActivate={(event) => {
          if (event.isTrusted) setOpen(true);
        }}
      />
      <ManagePage pageId={pageId} title={title} open={open} close={() => setOpen(false)} />
    </>
  );
}
function PageListActions({ page }: { page: PageSummary }) {
  const menu = useRef<HTMLDetailsElement>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const element = menu.current;
    if (!open || !element) return;
    const outside = (event: PointerEvent) => {
      // The management dialog is this disclosure's portal; keep its return target visible.
      if (event.target instanceof Element && event.target.closest('.management-dialog')) return;
      if (event.target instanceof Node && !element.contains(event.target)) element.open = false;
    };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open]);
  return (
    <details
      ref={menu}
      className="page-row-menu"
      onToggle={(event) => setOpen(event.currentTarget.open)}
      onKeyDown={(event) => {
        if (event.target instanceof Element && event.target.closest('.management-dialog')) return;
        if (event.key === 'Escape' && menu.current?.open) {
          menu.current.open = false;
          menu.current.querySelector('summary')?.focus();
          event.stopPropagation();
        }
      }}
      onBlur={(event) => {
        if (
          event.relatedTarget instanceof Element &&
          event.relatedTarget.closest('.management-dialog')
        )
          return;
        if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.open = false;
      }}
    >
      <summary aria-label={text.pageActionsFor(pageTitle(page))}>{text.pageActions}</summary>
      <div className="page-row-menu-body">
        <p>{text.askDetails}</p>
        <p className="management-id">Page ID: {page.id}</p>
        <RetentionHint page={page} />
        <ManageButton pageId={page.id} title={page.title} />
      </div>
    </details>
  );
}

function Home() {
  const space = home.useLoaderData();
  const { transport } = root.useRouteContext();
  const [archived, setArchived] = useState(false);
  const pages = orderPages(space.pages.filter((p) => Boolean(p.archived) === archived));
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => setNow(Date.now()), [space.pages, archived]);
  useEffect(() => {
    document.title =
      space.title === text.product ? text.product : `${space.title} · ${text.product}`;
  }, [space.title]);
  return (
    <section className="home">
      <p className="eyebrow">{text.pages}</p>
      <h1>{space.title}</h1>
      <p className="intro">{text.intro}</p>
      {transport.management && (
        <BrowserToggle
          pressed={archived}
          label={text.showArchived}
          onActivate={(event) => {
            if (event.isTrusted) setArchived(!archived);
          }}
        />
      )}
      {pages.length ? (
        <BrowserList label={text.pages} className="pages">
          {pages.map((p) => {
            const title = pageTitle(p);
            const updated = pageUpdate(p.lastUpdateAtMs, now);
            return (
              <BrowserListRow
                key={p.id}
                data-page-id={p.id}
                title={
                  p.archived ? (
                    <span title={title}>{title}</span>
                  ) : (
                    <Link to="/pages/$pageId" params={{ pageId: p.id }} title={title}>
                      {title}
                    </Link>
                  )
                }
                metadata={
                  updated.dateTime ? (
                    <time
                      dateTime={updated.dateTime}
                      title={updated.absolute}
                      aria-label={`${text.lastUpdated}: ${updated.label}`}
                    >
                      {updated.label}
                    </time>
                  ) : (
                    updated.label
                  )
                }
                state={
                  p.archived ? text.archived : p.sharing === 'private' ? text.private : text.shared
                }
                actions={transport.management ? <PageListActions page={p} /> : undefined}
              />
            );
          })}
        </BrowserList>
      ) : (
        <p>
          {archived ? (
            'No archived pages.'
          ) : space.pages.length ? (
            'No active pages.'
          ) : (
            <>
              {text.empty}
              <code className="tmt-ui-code">tmt colab page create --title "Notes"</code>.
            </>
          )}
        </p>
      )}
    </section>
  );
}
function Page() {
  const { transport } = root.useRouteContext();
  const snapshot = page.useLoaderData();
  const backendName = transport.backendName?.trim();
  const backendLabel = backendName ? `local · ${backendName}` : 'local';
  const toolbar = useRef<HTMLElement>(null);
  const [panel, setPanel] = useState<PageHeaderAction | null>(null);
  const [chatOpened, setChatOpened] = useState(false);
  const toggle = (value: PageHeaderAction) => {
    if (value === 'chat') setChatOpened(true);
    setPanel((previous) => (previous === value ? null : value));
  };
  const [view, setView] = useState<PageView>({
    source: snapshot.source,
    title: snapshot.title,
    originalAuthor: snapshot.originalAuthor,
    publisherAgent: snapshot.publisherAgent,
    ownData: snapshot.ownData ?? false,
    own: snapshot.own,
    asks: snapshot.asks,
    threads: snapshot.threads,
  });
  useEffect(() => {
    document.title = `${view.title || snapshot.title} · ${text.product}`;
    return () => {
      document.title = text.product;
    };
  }, [view.title, snapshot.title]);
  const latest = useRef(view),
    dirty = useRef(false),
    base = useRef(snapshot.source);
  const [draft, setDraft] = useState(snapshot.source),
    [saving, setSaving] = useState(false);
  const [liveError, setLiveError] = useState<Error | null>(null),
    [eviction, setEviction] = useState<SessionEvictedError | null>(null),
    [editError, setEditError] = useState<SaveProblem | null>(null);
  const recoveryRequired = liveError instanceof RecoveryRequiredError;
  const recoverySelection = useRef<{ node: HTMLElement; range: Range; backward: boolean } | null>(
    null,
  );
  function keepRecoverySelection() {
    if (!recoveryRequired) return;
    const node = document.activeElement;
    const selection = document.getSelection();
    if (
      !(node instanceof HTMLElement) ||
      !node.matches('[contenteditable="true"]') ||
      !node.closest('.annotation-compose') ||
      !selection?.rangeCount
    )
      return;
    const range = selection.getRangeAt(0);
    if (!node.contains(range.startContainer) || !node.contains(range.endContainer)) return;
    recoverySelection.current = {
      node,
      range: range.cloneRange(),
      backward:
        selection.anchorNode === range.endContainer && selection.anchorOffset === range.endOffset,
    };
  }
  useEffect(() => {
    keepRecoverySelection();
  }, [recoveryRequired]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    recoverySelection.current = null;
    dirty.current = false;
    base.current = snapshot.source;
    setDraft(snapshot.source);
    setView({
      source: snapshot.source,
      title: snapshot.title,
      originalAuthor: snapshot.originalAuthor,
      publisherAgent: snapshot.publisherAgent,
      ownData: snapshot.ownData ?? false,
      own: snapshot.own,
      asks: snapshot.asks,
      threads: snapshot.threads,
    });
    setLiveError(null);
    setEviction(null);
    setEditError(null);
    const unsubscribe = snapshot.binding?.subscribe(
      (value) => {
        const next = { ...value, ownData: value.ownData ?? false };
        latest.current = next;
        setView(next);
        if (!dirty.current) {
          base.current = value.source;
          setDraft(value.source);
        }
      },
      (error) => {
        void buildWatch.check();
        setLiveError(error);
        setEviction(error instanceof SessionEvictedError ? error : null);
      },
    );
    return () => {
      unsubscribe?.();
    };
  }, [snapshot]);
  async function save() {
    if (!snapshot.binding || saving) return;
    setSaving(true);
    setEditError(null);
    try {
      await snapshot.binding.edit(draft, base.current);
      dirty.current = false;
      base.current = latest.current.source;
      setDraft(latest.current.source);
    } catch (error) {
      setEditError({
        message: saveMessage(error),
        unconfirmed: error instanceof SaveOutcomeUnknown,
      });
    } finally {
      setSaving(false);
    }
  }
  const [reconnecting, setReconnecting] = useState(false);
  const [reconnectFailed, setReconnectFailed] = useState(false);
  async function reconnect() {
    if (reconnecting) return;
    const interrupted = liveError;
    setReconnecting(true);
    setReconnectFailed(false);
    try {
      if (await snapshot.binding?.reconnect?.()) {
        setLiveError((error) => (error === interrupted ? null : error));
      } else setReconnectFailed(true);
    } catch {
      setReconnectFailed(true);
    } finally {
      if (document.activeElement === recoverySelection.current?.node)
        recoverySelection.current = null;
      setReconnecting(false);
    }
  }
  const [selector, setSelector] = useState<QuoteSelector | null>(null);
  const currentSelector = useRef<QuoteSelector | null>(null);
  const [rectangle, setRectangle] = useState<SelectionRect | null>(null);
  const currentRectangle = useRef<SelectionRect | null>(null);
  const [annotation, setAnnotation] = useState<{
    key: string;
    selector: QuoteSelector | null;
    rectangle: SelectionRect;
    thread?: DiscussionRef;
    /** Text kept from an earlier close of this selection or thread. */
    restored?: ComposerEdit;
  }>();
  const binding = useRef(snapshot.binding);
  binding.current = snapshot.binding;
  const annotationRef = useRef(annotation);
  annotationRef.current = annotation;
  const popover = useRef<HTMLElement>(null);
  // An unsent draft lives in memory for this page only: never stored, never sent to the frame.
  const drafts = useRef(new Map<string, ComposerEdit>());
  // Device-local persistence behind `drafts`: it restores text only and never sends or opens.
  const session = useRef<DraftSession | undefined>(undefined);
  const [draftsReady, setDraftsReady] = useState(!transport.drafts);
  const [draftsUnsaved, setDraftsUnsaved] = useState(false);
  const [, setDraftKeys] = useState(0);
  function keepDraft(key: string, edit: ComposerEdit | null) {
    const had = drafts.current.has(key);
    if (edit) drafts.current.set(key, edit);
    else drafts.current.delete(key);
    session.current?.set(key, edit);
    if (had !== !!edit) setDraftKeys((n) => n + 1);
  }
  const annotationDraft = useRef<ComposerEdit>({ value: '' });
  const annotationBusy = useRef(false);
  const statusBusy = useRef(false);
  const [changingStatus, setChangingStatus] = useState(false);
  const [activeThread, setActiveThread] = useState<string | null>(null);
  useEffect(() => {
    setAnnotation(undefined);
    drafts.current.clear();
    annotationDraft.current = { value: '' };
    annotationBusy.current = statusBusy.current = false;
    setChangingStatus(false);
    setSelector(null);
    currentSelector.current = null;
    currentRectangle.current = null;
    setRectangle(null);
    setResolved([]);
    setAnchorsChecked(false);
    setActiveThread(null);
    setPanel(null);
    setChatOpened(false);
  }, [snapshot.id]);
  useEffect(() => {
    const store = transport.drafts;
    setDraftsUnsaved(false);
    if (!store) {
      setDraftsReady(true);
      return;
    }
    setDraftsReady(false);
    const current = new DraftSession(store, snapshot.id, setDraftsUnsaved);
    session.current = current;
    let live = true;
    void current.load().then((saved) => {
      if (!live) return;
      // Text typed before the read finished is newer than anything stored.
      for (const [key, edit] of saved ?? [])
        if (!drafts.current.has(key)) drafts.current.set(key, edit);
      setDraftKeys((n) => n + 1);
      setDraftsReady(true);
    });
    const leave = () => void current.flush();
    const hide = () => document.visibilityState === 'hidden' && leave();
    window.addEventListener('pagehide', leave);
    document.addEventListener('visibilitychange', hide);
    return () => {
      live = false;
      window.removeEventListener('pagehide', leave);
      document.removeEventListener('visibilitychange', hide);
      session.current = undefined;
      void current.close();
    };
  }, [snapshot.id, transport.drafts]);
  function annotate() {
    if (
      annotationBusy.current ||
      statusBusy.current ||
      !currentSelector.current ||
      !currentRectangle.current
    )
      return;
    closeRef.current(false);
    setAnnotation({
      key: crypto.randomUUID(),
      selector: structuredClone(currentSelector.current),
      rectangle: { ...currentRectangle.current },
      restored: drafts.current.get(JSON.stringify(currentSelector.current)),
    });
    setActiveThread(null);
    setRectangle(null);
    setPanel(null);
  }
  const annotationThread = annotation?.thread
    ? view.threads?.find(
        (thread) =>
          thread.ref.writer === annotation.thread!.writer &&
          thread.ref.id === annotation.thread!.id,
      )
    : undefined;
  const openThreads = openThreadCount(view.threads ?? []);
  const fileCount = view.attachments?.length ?? 0;
  const unseenThreads = (view.threadPresentations ?? []).some((value) => value.status.unseen);
  const statusCoordinator =
    liveError?.message === managementChanged ? undefined : snapshot.binding?.status;
  const annotationKey = (value: NonNullable<typeof annotation>) =>
    value.thread ? `${value.thread.writer}:${value.thread.id}` : JSON.stringify(value.selector);
  /** Every nonblank message is a draft; recipient selection never replaces its bytes. */
  const typed = () =>
    annotationDraft.current.edited !== false && annotationDraft.current.value.trim() !== '';
  /** Closes without losing typed text: it comes back when the same selection is annotated again. */
  function closeAnnotation(focusPage: boolean) {
    // A send in flight is not interrupted by the ×, Escape, an outside press or a cleared selection.
    if (!annotation || annotationBusy.current || statusBusy.current) return;
    const key = annotationKey(annotation);
    keepDraft(key, typed() || annotationDraft.current.edited ? annotationDraft.current : null);
    annotationDraft.current = { value: '' };
    setAnnotation(undefined);
    setRectangle(currentRectangle.current);
    if (focusPage) queueMicrotask(() => host.current?.querySelector('iframe')?.focus());
  }
  const closeRef = useRef(closeAnnotation);
  closeRef.current = closeAnnotation;
  const selectionCleared = useRef<() => void>(() => {});
  selectionCleared.current = () => {
    // A new selection keeps typed text; a saved window collapses with its draft cached.
    if (annotation && (annotation.thread || !typed())) closeAnnotation(false);
  };
  useEffect(() => {
    if (!annotation) return;
    const outside = (event: PointerEvent) => {
      const target = event.target;
      if (
        target instanceof Element &&
        !popover.current?.contains(target) &&
        !target.closest('[role="listbox"]') &&
        !(recoveryRequired && target.closest('[data-colab-reconnect]'))
      )
        closeRef.current(false);
    };
    document.addEventListener('pointerdown', outside, true);
    return () => document.removeEventListener('pointerdown', outside, true);
  }, [annotation, recoveryRequired]);
  function openThread(ref: DiscussionRef | null) {
    if (annotationBusy.current || statusBusy.current) return;
    if (!ref) {
      setActiveThread(null);
      return;
    }
    const id = `${ref.writer}:${ref.id}`;
    if (annotationRef.current) closeRef.current(false);
    setAnnotation(undefined);
    binding.current?.markThreadStatusSeen?.(ref);
    setActiveThread(id);
    setPanel('comments');
    if (location.pathname.startsWith('/p/'))
      history.replaceState(history.state, '', `${location.pathname}#t=${ref.id}`);
    renderer.current?.scrollAnchor(id);
  }
  function continueAnnotation(ref: DiscussionRef) {
    keepDraft(JSON.stringify(annotationRef.current?.selector), null);
    setAnnotation((previous) =>
      previous ? { ...previous, thread: ref, restored: undefined } : previous,
    );
  }
  function openAnchoredThread(ref: DiscussionRef) {
    if (annotationBusy.current || statusBusy.current) return;
    const thread = latest.current.threads?.find(
      (value) => value.ref.writer === ref.writer && value.ref.id === ref.id && !value.deleted,
    );
    if (!thread?.anchor) return;
    closeRef.current(false);
    binding.current?.markThreadStatusSeen?.(ref);
    renderer.current?.scrollAnchor(`${ref.writer}:${ref.id}`);
    const rectangle = renderer.current?.anchorRectangle(`${ref.writer}:${ref.id}`);
    if (!rectangle) {
      openThread(ref);
      return;
    }
    setAnnotation({
      key: crypto.randomUUID(),
      selector: structuredClone(thread.anchor),
      rectangle,
      thread: ref,
      restored: drafts.current.get(`${ref.writer}:${ref.id}`),
    });
    setActiveThread(null);
    setPanel(null);
  }
  const [resolved, setResolved] = useState<string[]>([]);
  const [anchorsChecked, setAnchorsChecked] = useState(false);
  const renderer = useRef<Awaited<ReturnType<typeof mountRenderer>> | null>(null);
  const [state, setState] = useState<RenderState | 'loading'>('loading');
  const deepThread = useRef<string | null>(null);
  useEffect(() => {
    if (state !== 'ready' || !location.pathname.startsWith('/p/')) return;
    const target = new URLSearchParams(location.hash.slice(1)).get('t');
    if (!target || deepThread.current === `${snapshot.id}:${target}`) return;
    const thread = view.threads?.find((value) => value.threadId === target && !value.deleted);
    if (!thread) return;
    deepThread.current = `${snapshot.id}:${target}`;
    setActiveThread(`${thread.ref.writer}:${thread.ref.id}`);
    setPanel('comments');
    renderer.current?.scrollAnchor(`${thread.ref.writer}:${thread.ref.id}`);
  }, [state, snapshot.id, view.threads]);

  // Author loading does not block discussion: sends use the captured quote.
  const discussionBlocked = !!liveError || state === 'failed' || state === 'navigation';
  const host = useRef<HTMLDivElement>(null);
  const proposalLayer = useProposalLayer({
    snapshot,
    view,
    binding,
    renderer,
    host,
    state,
    discussionBlocked,
    recoveryRequired,
    statusCoordinator,
    otherBusy: () => annotationBusy.current || statusBusy.current,
    closeAnnotation: () => closeRef.current(false),
    draft: (key) => drafts.current.get(key),
    keepDraft,
    activeThread,
    setActiveThread,
  });
  useEffect(() => {
    const controller = new AbortController();
    if (liveError) {
      host.current?.replaceChildren();
      setState('failed');
      return () => controller.abort();
    }
    setState('loading');
    // Source revisions replace only author content. Parent selection and composer
    // state keep the original quote and rectangle even if that quote is now stale.
    setResolved([]);
    setAnchorsChecked(false);
    void mountRenderer(host.current!, view.source, {
      signal: controller.signal,
      onState: setState,
      onSlots: proposalLayer.onSlots,
      viewportInset: () =>
        (toolbar.current?.offsetHeight ?? 0) + (toolbar.current?.getBoundingClientRect().top ?? 0),
      onSelection: (_value, quote, rect) => {
        setSelector(quote ?? null);
        currentSelector.current = quote ?? null;
        currentRectangle.current = rect ?? null;
        setRectangle(rect ?? null);
        if (!quote) selectionCleared.current();
      },
      onAnchors: (ids, checked = false) => {
        setResolved(ids);
        setAnchorsChecked(checked);
      },
      onAnnotate: annotate,
      onOpenThread: (id) => {
        const thread = latest.current.threads?.find(
          (value) => `${value.ref.writer}:${value.threadId}` === id && !value.deleted,
        );
        if (thread) openAnchoredThread(thread.ref);
      },
    })
      .then((handle) => {
        if (controller.signal.aborted) handle.destroy();
        else renderer.current = handle;
      })
      .catch(() => {
        if (!controller.signal.aborted) {
          host.current?.replaceChildren();
          setState('failed');
        }
      });
    return () => {
      // Retire messages before replacing the frame, retaining its layout and the
      // parent's frozen selection. Unmount removes the host itself.
      renderer.current?.release();
      controller.abort();
      renderer.current = null;
    };
  }, [view.source, liveError, proposalLayer.onSlots]);
  useEffect(() => {
    if (state !== 'ready') return;
    renderer.current?.highlight(
      // Resolving hides the margin marker; Reopen restores it. Resolved history stays in Comments.
      (view.threads ?? []).flatMap((thread) =>
        isStatusThread(thread) && !thread.resolved && thread.anchor
          ? [
              {
                id: `${thread.ref.writer}:${thread.threadId}`,
                selector: thread.anchor,
              },
            ]
          : [],
      ),
      annotation?.selector ?? undefined,
    );
  }, [state, view.threads, annotation]);
  return (
    <section
      className="page"
      onPointerDownCapture={keepRecoverySelection}
      onKeyDownCapture={(event) => {
        if (event.key === 'Escape' || event.key === 'Tab') keepRecoverySelection();
      }}
      onFocusCapture={(event) => {
        const kept = recoverySelection.current;
        if (!kept || event.target !== kept.node) return;
        recoverySelection.current = null;
        const { node, range, backward } = kept;
        if (
          !node.isConnected ||
          !node.contains(range.startContainer) ||
          !node.contains(range.endContainer)
        )
          return;
        // Mobile Chat must close its modal drawer to reach Reconnect. Restore
        // its retained selection only when this same composer regains focus.
        document
          .getSelection()
          ?.setBaseAndExtent(
            backward ? range.endContainer : range.startContainer,
            backward ? range.endOffset : range.startOffset,
            backward ? range.startContainer : range.endContainer,
            backward ? range.startOffset : range.endOffset,
          );
      }}
    >
      <ReplyAnnouncer key={snapshot.id} initialRecords={snapshot.asks} records={view.asks} />
      <ColabHeader
        headerRef={toolbar}
        title={view.title || snapshot.title || text.unknownPageTitle}
        caption={view.originalAuthor === undefined ? undefined : text.byAuthor(view.originalAuthor)}
        home={(brand) => (
          <Link to="/" aria-label={text.home}>
            {brand}
          </Link>
        )}
        status={
          <div className="page-header-meta">
            <span className="page-backend" title={backendLabel}>
              {backendLabel}
            </span>
            <span
              className={`chip page-sharing status ${state === 'ready' ? 'live' : ''}`}
              title={
                recoveryRequired
                  ? text.connectionLost
                  : state === 'ready'
                    ? text.loaded
                    : state === 'loading'
                      ? text.loading
                      : text.blocked
              }
            >
              {text[snapshot.sharing]} ·{' '}
              <span className="status-label">
                {recoveryRequired
                  ? text.connectionLost
                  : state === 'ready'
                    ? text.live
                    : state === 'loading'
                      ? text.loading
                      : text.blocked}
              </span>
            </span>
          </div>
        }
        actions={
          <PageHeaderActions
            active={panel}
            comments={openThreads}
            files={fileCount}
            unseen={unseenThreads}
            canExport={!!snapshot.binding && !liveError}
            canManage={!!transport.management}
            canFiles={!!snapshot.binding?.files}
            onSelect={toggle}
          />
        }
      />
      <ExportPanel
        key={`export:${snapshot.id}`}
        binding={snapshot.binding}
        blocked={!!liveError}
        drawer
        opened={panel === 'export'}
        changeOpen={(open) => setPanel(open ? 'export' : null)}
      />
      <ManagePage
        pageId={snapshot.id}
        title={view.title || snapshot.title}
        open={panel === 'manage'}
        close={() => setPanel(null)}
        changed={() => {
          snapshot.binding?.close();
          setLiveError(new Error(managementChanged));
        }}
      />
      <PageDrawer
        open={panel === 'about'}
        title={text.aboutPage}
        kind="about"
        close={() => setPanel(null)}
      >
        <PageAttribution
          originalAuthor={view.originalAuthor}
          publisherAgent={view.publisherAgent}
        />
        <p>{text.warning}</p>
      </PageDrawer>
      <div className="workspace">
        <div className="canvas">
          <div className="frame-host" ref={host} />
          {proposalLayer.inline}
          {(state === 'navigation' || state === 'failed') && (
            <NoticeCard
              state={recoveryRequired ? 'waiting' : 'blocked'}
              stateLabel={recoveryRequired ? text.disconnected : undefined}
              eyebrow={text.product}
              title={recoveryRequired ? text.connectionLost : text.blocked}
              actions={
                recoveryRequired &&
                snapshot.binding?.reconnect && (
                  <div
                    data-colab-reconnect
                    onPointerDown={(event) => {
                      // A pointer recovery action does not end the active draft's
                      // focus/selection. Keyboard activation keeps button policy.
                      if (
                        event.isTrusted &&
                        document.activeElement?.matches('[contenteditable="true"]')
                      )
                        event.preventDefault();
                    }}
                  >
                    <BrowserAction
                      type="button"
                      label={reconnecting ? text.reconnecting : text.reconnect}
                      variant="primary"
                      busy={reconnecting}
                      busyMark={<LoaderCircle />}
                      onActivate={(event) => {
                        if (event.isTrusted) void reconnect();
                      }}
                    />
                  </div>
                )
              }
            >
              {recoveryRequired ? (
                <p>{text.recoveryRequired}</p>
              ) : eviction ? (
                <>
                  <p>{text.sessionEvicted(eviction.limit)}</p>
                  <p>{text.sessionLimitCommand} </p>
                  <BrowserCommand
                    text={`tmt remote settings sessions-per-device ${eviction.limit + 1}`}
                  />
                  {eviction.settingsUrl && (
                    <p>
                      <a href={eviction.settingsUrl}>{text.remoteSettings}</a>
                    </p>
                  )}
                </>
              ) : (
                <>
                  {liveError ? (
                    <TerminalFailure error={liveError} />
                  ) : (
                    <p>{state === 'navigation' ? text.navigation : text.failed}</p>
                  )}
                  {!liveError &&
                    state === 'failed' &&
                    new TextEncoder().encode(view.source).length > MAX_RENDER_SOURCE_BYTES && (
                      <p>{text.limit}</p>
                    )}
                </>
              )}
              {!recoveryRequired && reconnectFailed && (
                <p>
                  <ReconnectFailure />
                </p>
              )}
            </NoticeCard>
          )}
        </div>
      </div>
      {snapshot.binding?.discussion &&
        (snapshot.binding?.ask || annotation) &&
        (annotation || (!liveError && state === 'ready')) && (
          <SelectionAnnotation
            host={host.current}
            rectangle={annotation?.rectangle ?? rectangle}
            inset={toolbar.current?.offsetHeight ?? 0}
            noticeVisible={state === 'navigation' || state === 'failed'}
            open={annotate}
          >
            {annotation && (
              <section
                key={annotation.key}
                className="annotation-new"
                role="dialog"
                aria-label="Annotate selection"
                ref={popover}
                onKeyDown={(event) => {
                  if (event.key === 'Escape' && !event.defaultPrevented) {
                    event.preventDefault();
                    closeAnnotation(true);
                  }
                }}
              >
                <ThreadWindow
                  thread={annotationThread}
                  anchor={annotation.selector ?? undefined}
                  layout="anchored"
                  attached={resolved.includes(
                    annotation.thread ? `${annotation.thread.writer}:${annotation.thread.id}` : '',
                  )}
                  anchorsChecked={anchorsChecked}
                  selection={selector}
                  binding={snapshot.binding.discussion}
                  ask={snapshot.binding.ask}
                  asks={view.asks ?? []}
                  title={view.title || snapshot.title}
                  close={() => {
                    if (annotationRef.current?.key === annotation.key) closeAnnotation(true);
                  }}
                  blocked={discussionBlocked}
                  observationUnavailable={view.askUnavailable}
                  status={
                    annotationThread
                      ? presentationOf(view.threadPresentations, annotationThread.ref)?.status
                      : undefined
                  }
                  onStatusChange={
                    annotationThread && statusCoordinator
                      ? (resolved) => statusCoordinator.change(annotationThread, resolved)
                      : undefined
                  }
                  onBusy={(busy) => {
                    if (annotationRef.current?.key !== annotation.key) return;
                    statusBusy.current = busy;
                    setChangingStatus(busy);
                  }}
                  composer={
                    <>
                      {annotation.restored !== undefined && (
                        <p className="annotation-hint">Draft kept</p>
                      )}
                      {draftsUnsaved && <p className="annotation-hint">{text.draftsNotSaved}</p>}
                      {annotationThread && <p className="annotation-reply-label">Reply</p>}
                      <AnnotationInput
                        creationRecipient={
                          annotationThread?.proposal?.proposer ?? view.creationRecipient
                        }
                        binding={snapshot.binding.ask}
                        discussion={snapshot.binding.discussion}
                        anchor={annotationThread ? annotationThread.anchor : annotation.selector}
                        thread={annotationThread}
                        asks={view.asks ?? []}
                        title={view.title || snapshot.title}
                        blocked={discussionBlocked || changingStatus || !!annotationThread?.deleted}
                        recoveryRequired={
                          recoveryRequired && !changingStatus && !annotationThread?.deleted
                        }
                        initialEdit={annotation.restored}
                        onDraft={(_value, edit) => {
                          if (annotationRef.current?.key !== annotation.key) return;
                          annotationDraft.current = edit;
                          keepDraft(annotationKey(annotation), edit);
                        }}
                        onBusy={(busy) => {
                          if (annotationRef.current?.key !== annotation.key) return;
                          annotationBusy.current = busy;
                        }}
                        cancel={() => closeAnnotation(true)}
                        committed={(ref) => {
                          if (annotationRef.current?.key === annotation.key)
                            continueAnnotation(ref);
                        }}
                      />
                    </>
                  }
                />
              </section>
            )}
          </SelectionAnnotation>
        )}
      {snapshot.binding?.files && (
        <PageDrawer
          open={panel === 'files'}
          title={text.files}
          kind="files"
          close={() => setPanel(null)}
        >
          <FilesPanel
            descriptors={view.attachments ?? []}
            files={snapshot.binding.files}
            disabled={!!liveError || saving}
          />
        </PageDrawer>
      )}
      <PageDrawer
        open={panel === 'source'}
        title={text.source}
        kind="source"
        close={() => setPanel(null)}
      >
        <div className="source">
          <span>
            <label htmlFor="source-edit">{text.source}</label>
            {snapshot.binding && (
              <button
                disabled={saving || !!liveError || draft === base.current}
                onClick={() => void save()}
              >
                {saving ? text.saving : text.save}
              </button>
            )}
          </span>
          {editError && <SaveNotice problem={editError} />}
          <textarea
            id="source-edit"
            readOnly={!snapshot.binding || saving || !!liveError}
            spellCheck={false}
            value={draft}
            onChange={(event) => {
              dirty.current = true;
              setDraft(event.target.value);
            }}
          />
        </div>
      </PageDrawer>
      <PageDrawer
        open={panel === 'comments'}
        title={text.comments}
        kind="comments"
        close={() => setPanel(null)}
      >
        <SavedDrafts
          drafts={savedDrafts(drafts.current, view.threads)}
          discard={(key) => keepDraft(key, null)}
        />
        {draftsUnsaved && (
          <p role="status" className="annotation-hint saved-drafts-notice">
            {text.draftsNotSaved}
          </p>
        )}
        <ThreadPanel
          reopenProposal={proposalLayer.reopenProposal}
          creationRecipient={view.creationRecipient}
          hideHeader
          renderProposal={proposalLayer.renderProposal}
          key={`discussion:${snapshot.id}:${draftsReady}`}
          threads={(view.threads ?? []).filter((thread) => !isChatThread(thread))}
          resolved={resolved}
          anchorsChecked={anchorsChecked}
          selection={selector}
          binding={
            liveError?.message === managementChanged ? undefined : snapshot.binding?.discussion
          }
          ask={liveError?.message === managementChanged ? undefined : snapshot.binding?.ask}
          title={view.title || snapshot.title}
          asks={view.asks ?? []}
          active={activeThread}
          select={openThread}
          presentations={view.threadPresentations}
          onStatusChange={
            statusCoordinator
              ? (thread, resolved) => statusCoordinator.change(thread, resolved)
              : undefined
          }
          onBusy={(busy) => {
            statusBusy.current = busy;
            setChangingStatus(busy);
          }}
          draft={(ref) => drafts.current.get(`${ref.writer}:${ref.id}`)}
          onDraft={(ref, edit) => keepDraft(`${ref.writer}:${ref.id}`, edit)}
          blocked={discussionBlocked}
        />
      </PageDrawer>
      <PageDrawer
        open={panel === 'agents'}
        title={text.agentStatus}
        kind="agents"
        close={() => setPanel(null)}
      >
        <AgentStatusPanel
          open={panel === 'agents'}
          binding={snapshot.binding?.ask}
          page={snapshot.id}
          admitted={!!snapshot.binding && !liveError}
        />
      </PageDrawer>
      <PageDrawer
        open={panel === 'chat'}
        title="Chat"
        kind="chat"
        hideHeader
        close={() => setPanel(null)}
      >
        {chatOpened && (
          <ChatPanel
            creationRecipient={view.creationRecipient}
            key={`chat:${snapshot.id}:${draftsReady}`}
            initialEdit={drafts.current.get(CHAT_DRAFT)}
            onDraft={(edit) => keepDraft(CHAT_DRAFT, edit)}
            unsaved={draftsUnsaved}
            observationUnavailable={view.askUnavailable}
            threads={view.threads ?? []}
            asks={view.asks ?? []}
            binding={liveError?.message === managementChanged ? undefined : snapshot.binding?.ask}
            discussion={
              liveError?.message === managementChanged ? undefined : snapshot.binding?.discussion
            }
            title={view.title || snapshot.title}
            blocked={discussionBlocked}
            recoveryRequired={recoveryRequired}
            close={() => setPanel(null)}
          />
        )}
      </PageDrawer>
    </section>
  );
}
export function createAppRouter(
  transport: PageTransport,
  space?: string,
  pageIds: readonly { pageId: string; deleted: boolean }[] = [],
) {
  let knownPages = pageIds;
  const history = space
    ? createBrowserHistory({
        parseLocation: () => {
          const fragment = new URLSearchParams(location.hash.slice(1));
          const prefix = location.pathname.startsWith('/p/') ? location.pathname.slice(3) : '';
          const matches = knownPages.filter((row) => row.pageId.startsWith(prefix));
          const target =
            matches.length === 1 && !matches[0].deleted
              ? `/pages/${matches[0].pageId}`
              : `/short/${prefix}`;
          let path = publicEntry()
            ? prefix
              ? target
              : '/'
            : fragment.get('space') === space
              ? (fragment.get('path') ?? '/')
              : '/blocked';
          if (!/^\/(?:pages\/[0-9a-f-]+|short\/[0-9a-f-]{8,36})?$/.test(path)) path = '/blocked';
          return {
            href: path,
            pathname: path,
            search: '',
            hash: '',
            state: { ...window.history.state, __TSR_index: window.history.state?.__TSR_index ?? 0 },
          };
        },
        createHref: (path) => {
          if (path === '/') return '/colab/';
          const id = path.replace(/^\/(?:pages|short)\//, '');
          if (!validPagePrefix(id)) return '/colab/';
          const current = location.pathname.startsWith('/p/') ? location.pathname.slice(3) : '';
          const matches = knownPages.filter((row) => row.pageId.startsWith(current));
          const target = path.startsWith('/pages/')
            ? validPagePrefix(current) && matches.length === 1 && matches[0].pageId === id
              ? current
              : shortPageId(
                  id,
                  knownPages.map((row) => row.pageId),
                )
            : id;
          const thread = target === current && location.hash.startsWith('#t=') ? location.hash : '';
          return `/p/${target}${thread}`;
        },
      })
    : createHashHistory();
  const router = createRouter({
    routeTree: root.addChildren([home, page, shortPage, blocked]),
    history,
    context: {
      transport,
      rememberPages: (pages) => {
        knownPages = pages;
      },
    },
  });
  router.subscribe('onResolved', () => void buildWatch.check());
  return router;
}
declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAppRouter>;
  }
}
