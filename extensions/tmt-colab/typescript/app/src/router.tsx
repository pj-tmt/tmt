import type { Selection } from './management.js';
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
import { validPagePrefix } from './short-links.js';
import { ArrowUpRight, LoaderCircle, Moon, MousePointer2, Sun } from 'lucide-react';
import { PageHeaderActions, type PageHeaderAction } from './page-header-actions.js';
import { ColabHeader } from './colab-header.js';
import { PageAttribution } from './page-attribution.js';
import { NoticeCard } from './notice-card.js';
import { RetentionHint } from './retention-hint.js';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
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
import { useQuestionLayer } from './question-layer.js';
import { useInlineSlots } from './inline-slots.js';
import { readQuestions } from './questions.js';
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
import { ConnectionOwner, reconnectAttempt } from './connection-owner.js';
import { PageReopener, readmittable, ReadmitBudget, retryableReopen } from './readmit.js';
import { terminalFailure } from './terminal-failure.js';
import { buildWatch } from './app-build.js';
import { SessionEvictedError } from './ask-remote.js';
import { RecoveryRequiredError } from './session-recovery.js';
import { ExportPanel } from './export-panel.js';
import { PageDrawer } from './page-drawer.js';
import { AgentStatusPanel } from './agent-status-panel.js';
import { AgentPresentation, useAgentPresentation } from './agent-directory.js';
import { ChatPanel } from './chat-panel.js';
import { FilesPanel } from './files-panel.js';
import { CreatorDefaultProvider } from './creator-default.js';
import { DraftSession } from './draft-store.js';
import { CHAT_DRAFT, SavedDrafts, savedDrafts } from './saved-drafts.js';
import { isChatThread } from './thread-records.js';
import { SaveOutcomeUnknown, saveMessage } from './save.js';
import { SaveNotice, type SaveProblem } from './save-notice.js';

/** The control belongs to trusted chrome; only mode is projected into author content. */
function SelectionMode({
  host,
  enabled,
  discussionOpen,
  panel,
  toggle,
}: {
  host: HTMLDivElement | null;
  enabled: boolean;
  discussionOpen: boolean;
  panel: PageHeaderAction | null;
  toggle(): void;
}) {
  const element = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const [placement, setPlacement] = useState<{
    right: number;
    bottom: number;
    visibility: 'visible' | 'hidden';
  }>({ right: 24, bottom: 24, visibility: 'hidden' });
  useEffect(() => {
    button.current?.setAttribute('aria-keyshortcuts', enabled ? 'c Escape' : 'c');
  }, [enabled]);
  useEffect(() => {
    const place = () => {
      const box = host?.getBoundingClientRect();
      if (!box) return;
      const margin = innerWidth <= 640 ? 16 : 24;
      // These parent-owned overlays sit outside the renderer host's layout box.
      // Their actual bounds keep the control clear without altering author geometry.
      const drawer = document
        .querySelector<HTMLDialogElement>('.page-drawer[open]')
        ?.getBoundingClientRect();
      const right = Math.min(box.right, drawer?.left ?? innerWidth);
      const bottom = Math.min(innerHeight, box.bottom);
      setPlacement({
        right: Math.max(margin, innerWidth - right + margin),
        bottom: innerHeight - bottom + margin,
        visibility: 'visible',
      });
    };
    place();
    const observer = new ResizeObserver(place);
    if (host) observer.observe(host);
    // Native dialog show/close happens in its own effect. Follow its actual layout,
    // rather than depending on sibling effect order.
    for (const drawer of document.querySelectorAll('.page-drawer')) observer.observe(drawer);
    const composer = document.querySelector('.annotation-popover');
    if (composer) observer.observe(composer);
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, { passive: true });
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place);
    };
  }, [host, enabled, panel, discussionOpen]);
  return (
    <div ref={element} className="selection-mode" style={placement}>
      {enabled && !discussionOpen && (
        <p className="selection-mode-hint">{text.selectionModeHint}</p>
      )}
      <BrowserIconAction
        type="button"
        variant="primary"
        label={enabled ? text.stopSelecting : text.selectionMode}
        buttonRef={button}
        pressed={enabled}
        icon={
          <>
            <MousePointer2 />
            {enabled && (
              <>
                <span>{text.selecting}</span>
                <kbd>Esc</kbd>
              </>
            )}
          </>
        }
        onActivate={(event) => {
          if (event.isTrusted) toggle();
        }}
      />
    </div>
  );
}

function SelectionAnnotation({
  host,
  rectangle,
  inset,
  noticeVisible,
  toggleVisible,
  open,
  children,
}: {
  host: HTMLDivElement | null;
  rectangle: SelectionRect | null;
  inset: number;
  noticeVisible: boolean;
  toggleVisible: boolean;
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
      // Parent chrome reserves the toggle's bottom strip only for overlapping windows.
      const margin = innerWidth <= 640 ? 16 : 24;
      const leftEdge =
        rectangle && frame
          ? Math.max(
              8,
              Math.min(
                innerWidth - width - 8,
                frame.right - width - 8,
                frame.left + Math.max(0, Math.min(frame.width, rectangle.x)),
              ),
            )
          : (element.current?.getBoundingClientRect().left ?? innerWidth - width - 8);
      const toggle = document.querySelector('.selection-mode')?.getBoundingClientRect();
      const overlaps =
        toggleVisible &&
        expanded &&
        toggle &&
        leftEdge + width > toggle.left &&
        leftEdge < toggle.right;
      const edge = overlaps
        ? Math.min(innerHeight, host?.getBoundingClientRect().bottom ?? innerHeight) -
          margin -
          40 -
          8
        : innerHeight - 8;
      const placementHeight = expanded ? Math.min(480, edge - inset - 4) : 38;
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
                  Math.min(edge - placementHeight, element.current!.getBoundingClientRect().top),
                );
          return {
            left: Math.max(8, Math.min(innerWidth - width - 8, previous.left)),
            top,
            maxHeight: Math.max(0, edge - top),
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
      const clampAnchor = (value: number) => Math.max(inset + 4, Math.min(edge, value));
      const top = expanded ? clampAnchor(selectionTop) : selectionTop;
      const bottom = expanded ? clampAnchor(selectionBottom) : selectionBottom;
      const beside = !expanded && right + width + 8 <= Math.min(frame.right, innerWidth - 8);
      const below = bottom + placementHeight + 8 <= edge;
      // Choose the edge using stable placement clearance, not the changing content
      // height. CSS fits the shell; first Send grows away from the same anchor edge.
      const above = expanded && !below && top - placementHeight - 6 >= inset + 4;
      const windowTop = Math.max(
        inset + 4,
        Math.min(
          edge - placementHeight,
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
          : { top: windowTop, maxHeight: Math.max(0, edge - windowTop) }),
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
  }, [host, rectangle, inset, expanded, noticeVisible, toggleVisible, children]);
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

const root = createRootRouteWithContext<{ transport: PageTransport; space?: string }>()({
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
  loader: ({ context }) => context.transport.spaceHome(),
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
  remote,
}: {
  linked?: boolean;
  title?: string;
  remote?: { deviceName?: string };
}) {
  return (
    <ColabHeader
      title={title}
      remote={remote}
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
  const { transport } = root.useRouteContext();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const isPage = pathname.startsWith('/pages/');
  return (
    <>
      {!isPage && (
        <AppHeader
          remote={pathname === '/' ? { deviceName: transport.backendName } : undefined}
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
  changed?(operation: Selection['operation']): void;
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
          committed={(operation) => {
            touched.current = true;
            changed?.(operation);
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
                  <Link to="/pages/$pageId" params={{ pageId: p.id }} title={title}>
                    {title}
                  </Link>
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
  const { transport, space } = root.useRouteContext();
  const loaded = page.useLoaderData();
  // A reopened binding replaces the loaded snapshot until the route loads a newer one.
  const [reopened, setReopened] = useState<{
    from: typeof loaded;
    next: typeof loaded;
  } | null>(null);
  const snapshot = reopened?.from === loaded ? reopened.next : loaded;
  const toolbar = useRef<HTMLElement>(null);
  const [panel, setPanel] = useState<PageHeaderAction | null>(null);
  const [chatOpened, setChatOpened] = useState(false);
  const toggle = (value: PageHeaderAction) => {
    if (value === 'chat') setChatOpened(true);
    setPanel((previous) => (previous === value ? null : value));
  };
  const [view, setView] = useState<PageView>({
    archived: snapshot.archived,
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
  const agents = useAgentPresentation(
    snapshot.binding?.ask,
    snapshot.id,
    !!snapshot.binding && !liveError,
    JSON.stringify(
      (view.asks ?? []).map(({ writer, operationId }) => [writer, operationId]).sort(),
    ),
  );
  const archived = view.archived ?? snapshot.archived ?? false;
  const wasArchived = useRef(archived);
  const [archiveAnnouncement, setArchiveAnnouncement] = useState('');
  useEffect(() => {
    if (archived && !wasArchived.current) setArchiveAnnouncement(text.archiveAnnouncement);
    wasArchived.current = archived;
  }, [archived]);
  const recoveryRequired = liveError instanceof RecoveryRequiredError;
  const readmit = useRef(new ReadmitBudget());
  // A route reload brings its own binding: a reopened one and an attempt in flight end with it.
  const reopener = useRef(new PageReopener<typeof loaded>());
  useEffect(() => () => reopener.current.reset(), [loaded]);
  const liveErrorRef = useRef<Error | null>(null);
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
      archived: snapshot.archived,
      source: snapshot.source,
      title: snapshot.title,
      originalAuthor: snapshot.originalAuthor,
      publisherAgent: snapshot.publisherAgent,
      ownData: snapshot.ownData ?? false,
      own: snapshot.own,
      asks: snapshot.asks,
      threads: snapshot.threads,
    });
    liveErrorRef.current = null;
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
        // An epoch advance or authority recheck is not a lost grant: reopen the page for the
        // new epoch without a reload, retrying a lagging epoch. A refused fresh open, or an
        // exhausted budget, shows the card.
        if (readmittable(error) && !reopener.current.busy && readmit.current.take(Date.now())) {
          reopener.current.start((signal) => transport.page(loaded.id, signal), {
            retryable: (failure) => retryableReopen(failure),
            done: (result) => {
              if ('value' in result) {
                snapshot.binding?.close();
                setReopened({ from: loaded, next: result.value });
                return;
              }
              void buildWatch.check();
              liveErrorRef.current = result.error;
              setLiveError(result.error);
              setEviction(result.error instanceof SessionEvictedError ? result.error : null);
            },
          });
          return;
        }
        if (reopener.current.busy) return;
        void buildWatch.check();
        liveErrorRef.current = error;
        setLiveError(error);
        setEviction(error instanceof SessionEvictedError ? error : null);
      },
    );
    return () => {
      unsubscribe?.();
    };
  }, [snapshot, loaded, transport]);
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
        if (liveErrorRef.current === interrupted) liveErrorRef.current = null;
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
  // The connection owner reconnects an interrupted page on its own (#2572), silently: the card and
  // its Reconnect button are unchanged, and the button is the same attempt on demand.
  useEffect(() => {
    const binding = snapshot.binding;
    if (!recoveryRequired || !binding?.reconnect) return;
    const owner = new ConnectionOwner({
      attempt: reconnectAttempt({
        reconnect: async () => (await binding.reconnect?.()) ?? false,
        failure: () => liveErrorRef.current,
        recovered: (interrupted) => {
          liveErrorRef.current = null;
          setLiveError((error) => (error === interrupted ? null : error));
        },
        recoverable: (failure) => failure instanceof RecoveryRequiredError,
      }),
    });
    void owner.reconnect();
    return () => owner.stop();
  }, [recoveryRequired, snapshot.binding]);
  const [selectionMode, setSelectionMode] = useState(false);
  useEffect(() => {
    if (!archived) return;
    setSelectionMode(false);
    setPanel((current) => (current === 'agents' || current === 'export' ? null : current));
  }, [archived]);
  const toggleSelectionMode = useRef<() => void>(() => {});
  const escapeAnnotation = useRef<() => void>(() => {});
  const chatBusy = useRef(false);
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
    setSelectionMode(false);
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
    archived || liveError?.message === managementChanged ? undefined : snapshot.binding?.status;
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
  // Author loading does not block discussion: sends use the captured quote.
  const discussionBlocked = archived || !!liveError || state === 'failed' || state === 'navigation';
  const canSelect = !!snapshot.binding?.discussion && !discussionBlocked && state === 'ready';
  escapeAnnotation.current = () => {
    if (annotationRef.current) closeRef.current(true);
    else if (panel === 'chat') {
      if (!chatBusy.current) setPanel(null);
    } else if (canSelect) setSelectionMode(false);
  };
  toggleSelectionMode.current = () => {
    if (canSelect) setSelectionMode((enabled) => !enabled);
  };
  useEffect(() => {
    renderer.current?.selectionMode(canSelect && selectionMode);
  }, [canSelect, selectionMode]);
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      const target = event.target;
      const editing =
        target instanceof HTMLElement &&
        (target.isContentEditable || !!target.closest('input,textarea,select,[role="textbox"]'));
      if (
        !event.isTrusted ||
        event.defaultPrevented ||
        editing ||
        event.repeat ||
        event.isComposing ||
        event.keyCode === 229 ||
        (event.code !== 'KeyC' && event.key !== 'Escape') ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey ||
        !canSelect
      )
        return;
      event.preventDefault();
      if (event.key === 'Escape') escapeAnnotation.current();
      else if (!selectionMode && currentSelector.current && currentRectangle.current) annotate();
      else toggleSelectionMode.current();
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [canSelect, selectionMode]);
  const host = useRef<HTMLDivElement>(null);
  const questions = useMemo(() => readQuestions(view.source), [view.source]);
  const inlineSlots = useInlineSlots({
    renderer,
    host,
    state,
    source: view.source,
    identity: `${(view.threads ?? [])
      .filter((thread) => thread.proposal && !thread.deleted)
      .map((thread) => thread.threadId)
      .join(',')}|${questions.map((question) => question.id).join(',')}`,
  });
  const questionLayer = useQuestionLayer({
    questions,
    snapshot,
    view,
    renderer,
    state,
    slots: inlineSlots.positions,
    blocked: discussionBlocked,
  });
  const proposalLayer = useProposalLayer({
    slotPositions: inlineSlots.positions,
    snapshot,
    view,
    binding,
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
      onSlots: inlineSlots.onSlots,
      onQuestions: questionLayer.onQuestions,
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
      onToggleSelectionMode: () => toggleSelectionMode.current(),
      onKey: (key) => {
        if (key === 'Escape') escapeAnnotation.current();
      },
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
  }, [view.source, liveError, inlineSlots.onSlots, questionLayer.onQuestions]);
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
  const content = (
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
        remote={{ deviceName: transport.backendName }}
        caption={
          view.originalAuthor === undefined ? undefined : text.headerByAuthor(view.originalAuthor)
        }
        home={(brand) => (
          <Link to="/" aria-label={text.home}>
            {brand}
          </Link>
        )}
        status={
          <div className="page-header-meta">
            <span
              className={`chip page-sharing status ${state === 'ready' ? 'live' : ''}`}
              title={
                archived
                  ? text.archiveReadOnlyHint
                  : recoveryRequired
                    ? text.connectionLost
                    : state === 'ready'
                      ? text.loaded
                      : state === 'loading'
                        ? text.loading
                        : text.blocked
              }
            >
              {archived ? '○ ' : `${text[snapshot.sharing]} · `}
              <span className="status-label">
                {archived
                  ? text.archiveReadOnly
                  : recoveryRequired
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
            readOnly={archived}
            canExport={!!snapshot.binding && !liveError && !archived}
            canManage={!!transport.management}
            canFiles={!!snapshot.binding?.files}
            onSelect={toggle}
          />
        }
      />
      <p role="status" className="archive-announcement">
        {archiveAnnouncement}
      </p>
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
        changed={(operation) => {
          // Archive commits invalidate sync; verified read catch-up keeps this draft owner mounted.
          if (operation === 'page.archive') return;
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
          {questionLayer.inline}
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
      {canSelect && (
        <SelectionMode
          host={host.current}
          enabled={selectionMode}
          discussionOpen={!!annotation}
          panel={panel}
          toggle={() => toggleSelectionMode.current()}
        />
      )}
      {snapshot.binding?.discussion &&
        (snapshot.binding?.ask || annotation) &&
        (annotation || !selectionMode) &&
        (annotation || (!liveError && state === 'ready')) && (
          <SelectionAnnotation
            host={host.current}
            rectangle={annotation?.rectangle ?? rectangle}
            inset={toolbar.current?.offsetHeight ?? 0}
            noticeVisible={state === 'navigation' || state === 'failed'}
            toggleVisible={canSelect}
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
                  if (
                    event.key === 'Escape' &&
                    event.isTrusted &&
                    !event.defaultPrevented &&
                    !event.repeat &&
                    !event.nativeEvent.isComposing &&
                    event.keyCode !== 229 &&
                    !event.altKey &&
                    !event.ctrlKey &&
                    !event.metaKey &&
                    !event.shiftKey
                  ) {
                    event.preventDefault();
                    escapeAnnotation.current();
                  }
                }}
              >
                <ThreadWindow
                  archived={archived}
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
                        archived={archived}
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
            readOnly={archived}
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
            {snapshot.binding && !archived && (
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
            readOnly={archived || !snapshot.binding || saving || !!liveError}
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
          archived={archived}
          drafts={savedDrafts(drafts.current, view.threads)}
          discard={(key) => keepDraft(key, null)}
        />
        {draftsUnsaved && (
          <p role="status" className="annotation-hint saved-drafts-notice">
            {text.draftsNotSaved}
          </p>
        )}
        <ThreadPanel
          archived={archived}
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
        escape={() => escapeAnnotation.current()}
      >
        {chatOpened && (
          <ChatPanel
            archived={archived}
            creationRecipient={view.creationRecipient}
            key={`chat:${snapshot.id}:${draftsReady}`}
            initialEdit={drafts.current.get(CHAT_DRAFT)}
            onBusy={(busy) => {
              chatBusy.current = busy;
            }}
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
            escape={() => escapeAnnotation.current()}
          />
        )}
      </PageDrawer>
    </section>
  );
  const creatorScope = JSON.stringify([
    space ?? '',
    snapshot.id,
    loaded.binding?.discussion?.deviceId ?? '',
  ]);
  return (
    <CreatorDefaultProvider key={creatorScope} scope={creatorScope}>
      <AgentPresentation value={agents}>{content}</AgentPresentation>
    </CreatorDefaultProvider>
  );
}
export function createAppRouter(transport: PageTransport, space?: string) {
  const history = space
    ? createBrowserHistory({
        parseLocation: () => {
          const fragment = new URLSearchParams(location.hash.slice(1));
          let path = fragment.get('space') === space ? (fragment.get('path') ?? '/') : '/blocked';
          if (!/^\/(?:pages\/[0-9a-f-]+|short\/[0-9a-f-]{8,36})?$/.test(path)) path = '/blocked';
          return {
            href: path,
            pathname: path,
            search: '',
            hash: '',
            state: { ...window.history.state, __TSR_index: window.history.state?.__TSR_index ?? 0 },
          };
        },
        createHref: (path) =>
          `${location.pathname}#space=${space}${path === '/' ? '' : `&path=${encodeURIComponent(path)}`}`,
      })
    : createHashHistory();
  const router = createRouter({
    routeTree: root.addChildren([home, page, shortPage, blocked]),
    history,
    context: { transport, space },
  });
  router.subscribe('onResolved', () => void buildWatch.check());
  return router;
}
declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAppRouter>;
  }
}
