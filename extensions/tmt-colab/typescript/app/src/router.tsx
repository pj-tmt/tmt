import { usePageLayers } from './page-layers.js';
import { RemotePresenceProvider } from './remote-header.js';
import './source-editor.css';
import { OwnPublicationTimeout } from './own-publication.js';
import type { ComposerEdit } from './components/message-composer-edit.js';
import {
  BrowserAction,
  BrowserCommand,
  BrowserIconAction,
  BrowserNotifications,
} from '@tmt/browser-ui/react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { validPagePrefix } from './short-links.js';
import { ownerReadingHint, ownerReadingHref, readingSearch } from './reading-link.js';
import { ArrowUpRight, MessageCircle, MessageSquare, Moon, MousePointer2, Sun } from 'lucide-react';
import { PageHeaderActions, type PageHeaderAction } from './page-header-actions.js';
import { escapeChapter, finishReturn, useOpenChapter } from './chapter.js';
import { ChapterBar, HeldToday } from './chapter-layer.js';
import { ColabHeader } from './colab-header.js';
import { PageAttribution } from './page-attribution.js';
import { NoticeCard } from './notice-card.js';
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
  type Ref,
} from 'react';
import {
  createHashHistory,
  createBrowserHistory,
  createRootRouteWithContext,
  createRoute,
  createRouter,
  Link,
  redirect,
  Outlet,
  useRouterState,
  useRouter,
} from '@tanstack/react-router';
import type { PageView, PageTransport } from './transport.js';
import { AnnotationInput } from './annotation-input.js';
import { useProposalLayer } from './proposal-layer.js';
import { rememberPages } from './quick-switcher-source.js';
import { noteOpened } from './recently-opened.js';
import { QuickSwitcherHost, QuickSwitcherModalPresence } from './quick-switcher-dialog.js';
import { useQuickSwitcher } from './quick-switcher-context.js';
import { NeedsYouHeader } from './needs-you-header.js';
import { pendingNeedsYou } from './needs-you.js';
import { useReplyNotifications } from './use-reply-notifications.js';
import { ReplyNotificationObserver } from './reply-notifications.js';
import { ReplyNotificationContext } from './reply-notification-context.js';
import { ThreadPanel, ThreadWindow } from './thread-panel.js';
import { presentationOf } from './thread-status-presentation.js';
import { isStatusThread, openThreadCount } from './thread-status-view.js';
import type { QuoteSelector, DiscussionRef } from './thread-records.js';
import { Home, validateHomeSearch } from './home.js';
import { Today } from './today.js';
import { ManagePage } from './manage-page.js';
import { mountRenderer, MAX_RENDER_SOURCE_BYTES } from './renderer.js';
import type { AnchorPosition, RenderState, SelectionRect } from './renderer.js';
import { text } from './strings.js';
import { getTheme, setThemeChoice, subscribeTheme } from './theme.js';
import type { ConnectionState, EndedAction } from './connection-model.js';
import { useConnection } from './use-connection.js';
import { terminalFailure } from './terminal-failure.js';
import { buildWatch } from './app-build.js';
import { SessionEvictedError } from './ask-remote.js';
import { SessionLimitError } from './registration.js';
import { SessionLimitBody } from './session-limit.js';
import { ExportPanel } from './export-panel.js';
import { BrowserDrawer } from '@tmt/browser-ui/react';
import { AgentStatusPanel } from './agent-status-panel.js';
import { AgentPresentation, useAgentPresentation } from './agent-directory.js';
import { ChatPanel } from './chat-panel.js';
import { AttachmentDropPanel } from './attachment-drop.js';
import { FilesPanel } from './files-panel.js';
import { CreatorDefaultProvider } from './creator-default.js';
import { DraftSession } from './draft-store.js';
import { CHAT_DRAFT, SavedDrafts, savedDrafts } from './saved-drafts.js';
import { isChatThread, threadNumbers } from './thread-records.js';
import { SaveConflict, SaveOutcomeUnknown, SaveRefused, saveMessage } from './save.js';
import { PageSizeNotice, PageSizeSection, usePageSize } from './page-size.js';
import { compactOffered } from './page-usage.js';
import { SaveNotice, type SaveProblem } from './save-notice.js';

/** In-page cards whose controls the floating actions move above, never over. */
const AVOIDED_CARDS = '.question-group-footer, .proposal-inline .proposal-card';

/** The control belongs to trusted chrome; only mode is projected into author content. */
function PageFloatingActions({
  host,
  enabled,
  discussionOpen,
  panel,
  toggle,
  chatUnread,
  chatButton,
  toggleChat,
  canSelect,
  slots,
}: {
  slots: readonly AnchorPosition[];
  host: HTMLDivElement | null;
  enabled: boolean;
  discussionOpen: boolean;
  panel: PageHeaderAction | null;
  toggle(): void;
  chatUnread: boolean;
  chatButton: Ref<HTMLButtonElement>;
  toggleChat(): void;
  canSelect: boolean;
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
        .querySelector<HTMLDialogElement>('.tmt-ui-drawer[open]')
        ?.getBoundingClientRect();
      const right = Math.min(box.right, drawer?.left ?? innerWidth);
      let bottom = Math.min(innerHeight, box.bottom);
      const fab = element.current?.getBoundingClientRect();
      const left = right - margin - (fab?.width ?? 0);
      const cards = [...(host?.parentElement?.querySelectorAll<HTMLElement>(AVOIDED_CARDS) ?? [])]
        .filter((footer) => !footer.closest('[data-detached],[hidden]'))
        .map((footer) => (footer.closest('.question-group-card') ?? footer).getBoundingClientRect())
        .filter(
          (footer) =>
            footer.width > 0 && footer.height > 0 && footer.bottom > 0 && footer.top < innerHeight,
        )
        .sort((a, b) => b.top - a.top);
      for (const footer of cards) {
        if (
          left < footer.right &&
          right - margin > footer.left &&
          bottom - margin > footer.top &&
          bottom - margin - (fab?.height ?? 0) < footer.bottom
        )
          bottom = footer.top + margin - 8;
      }
      setPlacement({
        right: Math.max(margin, innerWidth - right + margin),
        bottom: innerHeight - bottom + margin,
        visibility: bottom - margin - (fab?.height ?? 0) >= 8 ? 'visible' : 'hidden',
      });
    };
    place();
    const observer = new ResizeObserver(place);
    if (host) observer.observe(host);
    if (element.current) observer.observe(element.current);
    for (const card of host?.parentElement?.querySelectorAll(AVOIDED_CARDS) ?? [])
      observer.observe(card);
    // Native dialog show/close happens in its own effect. Follow its actual layout,
    // rather than depending on sibling effect order.
    for (const drawer of document.querySelectorAll('.tmt-ui-drawer')) observer.observe(drawer);
    const composer = document.querySelector('.annotation-popover');
    if (composer) observer.observe(composer);
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, { passive: true });
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place);
    };
  }, [host, enabled, panel, discussionOpen, slots]);
  return (
    <div ref={element} className="selection-mode" style={placement}>
      {enabled && !discussionOpen && (
        <p className="selection-mode-hint">{text.selectionModeHint}</p>
      )}
      <div className="page-floating-actions">
        {!enabled && !discussionOpen && (
          <BrowserIconAction
            type="button"
            variant="primary"
            label={chatUnread ? `${text.chat} · ${text.chatUnread}` : text.chat}
            buttonRef={chatButton}
            hasPopup="dialog"
            expanded={panel === 'chat'}
            icon={
              <>
                <MessageCircle />
                {chatUnread && (
                  <span className="chat-unread">
                    <span aria-hidden="true">•</span> {text.chatUnread}
                  </span>
                )}
              </>
            }
            onActivate={(event) => {
              if (event.isTrusted) toggleChat();
            }}
          />
        )}
        {canSelect && (
          <BrowserIconAction
            type="button"
            variant="primary"
            label={text.selectionMode}
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
        )}
      </div>
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
    leaderY?: number;
    leaderSide?: 'left' | 'right';
  } | null>(null);
  useEffect(() => {
    const notice = noticeVisible
      ? host?.parentElement?.querySelector<HTMLElement>('.tmt-ui-notice')
      : null;
    const place = () => {
      const frame = host?.querySelector('iframe')?.getBoundingClientRect();
      const width = expanded ? Math.min(440, innerWidth - 24) : 128;
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
      if (expanded && innerWidth <= 640) {
        setPosition({
          left: 12,
          bottom: Math.max(8, innerHeight - edge),
          maxHeight: Math.max(
            0,
            edge - Math.max(inset, notice?.getBoundingClientRect().bottom ?? inset) - 8,
          ),
        });
        return;
      }
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
      const beside = right + width + 24 <= Math.min(frame.right, innerWidth - 8);
      const besideLeft = !beside && expanded && left - width - 24 >= Math.max(frame.left, 8);
      const below = bottom + placementHeight + 8 <= edge;
      // Choose the edge using stable placement clearance, not the changing content
      // height. CSS fits the shell; first Send grows away from the same anchor edge.
      const above =
        expanded && !beside && !besideLeft && !below && top - placementHeight - 6 >= inset + 4;
      const windowTop = Math.max(
        inset + 4,
        Math.min(
          edge - placementHeight,
          beside || besideLeft ? top : below ? bottom + 6 : top - placementHeight - 6,
        ),
      );
      setPosition({
        left: Math.max(
          8,
          Math.min(
            innerWidth - width - 8,
            frame.right - width - 8,
            beside ? right + 24 : besideLeft ? left - width - 24 : left,
          ),
        ),
        ...((beside || besideLeft) && innerWidth > 640
          ? {
              leaderY: Math.max(
                6,
                Math.min(placementHeight - 6, top + rectangle.height / 2 - windowTop),
              ),
              leaderSide: beside ? ('left' as const) : ('right' as const),
            }
          : {}),
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
      data-leader={expanded ? undefined : position?.leaderSide}
      style={
        {
          ...(position
            ? {
                left: position.left,
                top: position.top,
                bottom: position.bottom,
                maxHeight: position.maxHeight,
              }
            : {}),
          '--annotation-leader-y': `${position?.leaderY ?? 0}px`,
          visibility: position ? 'visible' : 'hidden',
        } as React.CSSProperties
      }
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
          <MessageSquare aria-hidden />
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
function TerminalFailure({
  error,
  reference: showReference = true,
}: {
  error: Error;
  reference?: boolean;
}) {
  if (error.message === managementChanged) return <p>{managementChanged}</p>;
  const { sentence, reference } = terminalFailure(error);
  return (
    <>
      <p>{sentence === text.reconnectFailed ? <ReconnectFailure /> : sentence}</p>
      {showReference && reference && (
        <p>
          <small className="failure-reference">
            {text.failureCodeLabel}{' '}
            <code
              className="tmt-ui-code"
              data-failure-reference
              data-own-publication={
                error instanceof OwnPublicationTimeout && error.diagnostic
                  ? JSON.stringify(error.diagnostic)
                  : undefined
              }
            >
              {reference}
            </code>
          </small>
        </p>
      )}
    </>
  );
}

/** The sentence and the one action of an ended connection, shared by the page card and the drawer row; the row leaves out the failure code. */
function EndedBody({
  error,
  action,
  compact,
}: {
  error: Error;
  action: EndedAction;
  compact?: boolean;
}) {
  if (error instanceof SessionEvictedError)
    return <SessionLimitBody limit={error.limit} settingsUrl={error.settingsUrl} />;
  if (error instanceof SessionLimitError)
    return <SessionLimitBody refused limit={error.limit} settingsUrl={error.settingsUrl} />;
  return (
    <>
      <TerminalFailure error={error} reference={!compact} />
      {action === 'pair' && (
        <>
          <p>{text.pairAgain}</p>
          <BrowserCommand text={text.reconnectCommand} />
        </>
      )}
    </>
  );
}

function EndedLink({ action }: { action: EndedAction }) {
  if (action === 'reload')
    return (
      <BrowserAction
        type="button"
        variant="text"
        label={text.reload}
        onActivate={() => location.reload()}
      />
    );
  return action === 'pages' ? (
    <Link className={ui.action} data-variant="text" to="/">
      {text.retry}
    </Link>
  ) : null;
}

/** The page owner projects C1 once; composers do not classify transport errors. */
export type ComposerConnectionPresentation = {
  phase: 'reconnecting' | 'ended';
  body: ReactNode;
};
function composerConnection(state: ConnectionState): ComposerConnectionPresentation | undefined {
  if (state.phase === 'connected') return undefined;
  return {
    phase: state.phase,
    body:
      state.phase === 'reconnecting' ? (
        text.reconnecting
      ) : (
        <>
          <EndedBody error={state.error} action={state.action} compact />
          <EndedLink action={state.action} />
        </>
      ),
  };
}

const root = createRootRouteWithContext<{ transport: PageTransport; space?: string }>()({
  component: Shell,
  errorComponent: function RootError({ error }) {
    const header = useRef<HTMLElement>(null);
    useLayoutEffect(() => {
      const title = header.current?.querySelector('h1');
      if (title) {
        title.tabIndex = -1;
        title.focus({ preventScroll: true });
      }
    }, []);
    return (
      <>
        <AppHeader title={text.error} headerRef={header} />
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
    );
  },
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
async function loadSpaceHome({
  context,
}: {
  context: { transport: PageTransport; space?: string };
}) {
  const home = await context.transport.spaceHome();
  // A quick switcher opened later shows this list at once and refreshes it.
  rememberPages(context.transport, context.space, home.pages);
  return home;
}
const home = createRoute({
  getParentRoute: () => root,
  path: '/',
  // An old link or history entry that still carries list search belongs to the list.
  validateSearch: validateHomeSearch,
  beforeLoad: ({ search }) => {
    if (Object.values(search).some((value) => value !== undefined))
      throw redirect({ to: '/all', search, replace: true });
  },
  loader: loadSpaceHome,
  component: Today,
});
const allPages = createRoute({
  getParentRoute: () => root,
  path: '/all',
  validateSearch: validateHomeSearch,
  loader: loadSpaceHome,
  component: Home,
});
const page = createRoute({
  getParentRoute: () => root,
  path: '/pages/$pageId',
  validateSearch: readingSearch,
  loader: ({ context, params, abortController }) =>
    context.transport.page(params.pageId, abortController.signal),
  component: Page,
});

const shortPage = createRoute({
  getParentRoute: () => root,
  path: '/short/$prefix',
  validateSearch: readingSearch,
  loader: async ({ context, params, location }) => {
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
      throw redirect({
        to: '/pages/$pageId',
        params: { pageId: pages[0].id },
        search: readingSearch(location.search),
        replace: true,
      });
    return { pages, prefix: params.prefix };
  },
  component: ShortPageChoice,
});
function ShortPageChoice() {
  const { pages, prefix } = shortPage.useLoaderData();
  const search = shortPage.useSearch();
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
              <Link to="/pages/$pageId" params={{ pageId: page.id }} search={search}>
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
  headerRef,
}: {
  linked?: boolean;
  title?: string;
  remote?: { deviceName?: string };
  headerRef?: Ref<HTMLElement>;
}) {
  return (
    <ColabHeader
      title={title}
      headerRef={headerRef}
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
  const { transport, space } = root.useRouteContext();
  const router = useRouter();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const isPage = pathname.startsWith('/pages/');
  const isHome = pathname === '/' || pathname === '/all';
  // The chapter chrome follows the page the outlet shows (see `useOpenChapter`).
  const open = useOpenChapter();
  const opened = open?.chapter;
  const mode = open?.mode;
  const pageId = open?.pageId;
  return (
    <RemotePresenceProvider owner={transport.remotePresence}>
      <QuickSwitcherHost
        transport={transport}
        space={space}
        pageId={isPage ? pathname.slice('/pages/'.length) : undefined}
        routeKey={pathname}
        allowed={isPage || isHome}
        navigate={(pageId) => router.navigate({ to: '/pages/$pageId', params: { pageId } })}
        pageHref={(pageId) =>
          router.history.createHref(
            router.buildLocation({ to: '/pages/$pageId', params: { pageId } }).publicHref,
          )
        }
      >
        {!isPage && (
          <AppHeader
            remote={isHome ? { deviceName: transport.backendName } : undefined}
            title={
              pathname.startsWith('/short/')
                ? 'Open a page'
                : pathname === '/'
                  ? text.today.title
                  : pathname === '/all'
                    ? text.pages
                    : text.error
            }
          />
        )}
        {mode === 'panel' && pageId && opened && <HeldToday pageId={pageId} chapter={opened} />}
        <main className={isPage ? 'page-main' : undefined} data-chapter={mode}>
          {mode && pageId && opened && <ChapterBar mode={mode} pageId={pageId} chapter={opened} />}
          <Outlet />
        </main>
        {!isPage && (
          <footer>{location.pathname.startsWith('/r/') ? text.mountedNote : text.adapter}</footer>
        )}
      </QuickSwitcherHost>
    </RemotePresenceProvider>
  );
}
function Page() {
  const router = useRouter();
  const quickSwitcher = useQuickSwitcher();
  const quickSwitcherOwner = useRef(quickSwitcher);
  quickSwitcherOwner.current = quickSwitcher;
  const { transport, space } = root.useRouteContext();
  const loaded = page.useLoaderData();
  // A reopened binding replaces the loaded snapshot until the route loads a newer one.
  const [reopened, setReopened] = useState<{
    from: typeof loaded;
    next: typeof loaded;
  } | null>(null);
  const snapshot = reopened?.from === loaded ? reopened.next : loaded;
  const replyObserver = useMemo(() => new ReplyNotificationObserver(snapshot), [snapshot]);
  const toolbar = useRef<HTMLElement>(null);
  const [panel, setPanel] = useState<PageHeaderAction | null>(null);
  const [chatOpened, setChatOpened] = useState(false);
  const floatingChat = useRef<HTMLButtonElement>(null);
  const [chatActivation, setChatActivation] = useState<{ id: string; insertMention: boolean }>();
  const activateChat = useRef<() => boolean>(() => false);
  useEffect(() => {
    if (panel !== 'chat') setChatActivation(undefined);
  }, [panel]);
  useEffect(() => setChatActivation(undefined), [snapshot.binding]);

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
  // This device remembers which pages it opened, for a quick switcher to offer first; local only.
  useEffect(() => {
    void noteOpened(space, snapshot.id);
  }, [space, snapshot.id]);
  const latest = useRef(view),
    dirty = useRef(false),
    base = useRef(snapshot.source);
  const [draft, setDraft] = useState(snapshot.source),
    [saving, setSaving] = useState(false);
  const [editError, setEditError] = useState<SaveProblem | null>(null);
  /** The last Save merged the edit into a page that had changed; true until the next edit. */
  const [merged, setMerged] = useState(false);
  /** How the last Copy draft ended. */
  const [copied, setCopied] = useState<'copied' | 'failed' | null>(null);
  const sourceArea = useRef<HTMLTextAreaElement>(null);
  /** Where the caret and scroll were when a merged Save replaced the text, to put them back. */
  const placement = useRef<{ start: number; end: number; top: number } | null>(null);
  const size = usePageSize(snapshot.binding);
  const { state: connection, controller: connectionOwner } = useConnection({
    loaded,
    binding: () => snapshot.binding,
    reopen: (signal) => transport.page(loaded.id, signal),
    // A reopened binding replaces the loaded snapshot until the route loads a newer one.
    opened: (next) => setReopened({ from: loaded, next }),
    failed: () => void buildWatch.check(),
  });
  // Anything not connected holds writes; only an ended connection replaces the page with a card.
  const liveError = connection.phase === 'connected' ? null : connection.error;
  const endedError = connection.phase === 'ended' ? connection.error : null;
  const recoveryRequired = connection.phase === 'reconnecting';
  const connectionPresentation = composerConnection(connection);
  const endedAction = connection.phase === 'ended' ? connection.action : 'none';
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
  useEffect(() => {
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
    connectionOwner.reset();
    setEditError(null);
    let firstView = true;
    const unsubscribe = snapshot.binding?.subscribe(
      (value) => {
        const next = { ...value, ownData: value.ownData ?? false };
        if (firstView) {
          firstView = false;
          replyObserver.baselineFirstView(next);
        }
        latest.current = next;
        setView(next);
        if (!dirty.current) {
          base.current = value.source;
          setDraft(value.source);
        }
      },
      (error) => connectionOwner.report(error),
    );
    return () => {
      unsubscribe?.();
    };
  }, [snapshot, loaded, transport, connectionOwner, replyObserver]);
  async function save() {
    if (!snapshot.binding || saving) return;
    setSaving(true);
    setEditError(null);
    setMerged(false);
    setCopied(null);
    const area = sourceArea.current;
    try {
      const outcome = await snapshot.binding.edit(draft, base.current);
      dirty.current = false;
      base.current = latest.current.source;
      if (outcome.merged && area && document.activeElement === area)
        placement.current = {
          start: area.selectionStart,
          end: area.selectionEnd,
          top: area.scrollTop,
        };
      setDraft(latest.current.source);
      setMerged(outcome.merged);
      void size.refresh();
    } catch (error) {
      const problem: SaveProblem = {
        message: saveMessage(error),
        unconfirmed: error instanceof SaveOutcomeUnknown,
        ...(error instanceof SaveConflict ? { conflict: true as const } : {}),
      };
      // A full page names the way out that can help, decided by what the serve says is limiting
      // it now: combining changes frees room only when the changes are the limit.
      if (error instanceof SaveRefused && error.code === 'COLAB_CAPACITY') {
        const usage = await size.refresh();
        // A device that cannot ask the serve to combine is told who can.
        if (!snapshot.binding.compact) problem.message = text.pageSize.saveFullReader;
        else if (usage) {
          problem.full = compactOffered(usage) ? 'compact' : 'export';
          problem.message = compactOffered(usage)
            ? text.pageSize.saveFullTail
            : text.pageSize.saveFullBudget;
        }
      }
      setEditError(problem);
    } finally {
      setSaving(false);
    }
  }
  // A merged Save replaces the text with the merged page; the caret and scroll stay where the new
  // text allows, and nothing takes focus.
  useLayoutEffect(() => {
    const area = sourceArea.current,
      kept = placement.current;
    placement.current = null;
    if (!area || !kept || document.activeElement !== area) return;
    area.setSelectionRange(Math.min(kept.start, draft.length), Math.min(kept.end, draft.length));
    area.scrollTop = kept.top;
  }, [draft]);
  /** Copy the unsaved draft from a conflict, since a reload loses it. */
  function copyDraft() {
    void navigator.clipboard.writeText(draft).then(
      () => setCopied('copied'),
      () => setCopied('failed'),
    );
  }
  /** Compact from the save refusal: the draft stays, nothing is saved or resent; the person
   * presses Save again. */
  async function compactFromRefusal() {
    const outcome = await size.compact();
    setEditError(
      outcome.kind === 'failed'
        ? { message: text.pageSize.failed, unconfirmed: false, full: 'compact' }
        : outcome.kind === 'nothing'
          ? { message: text.pageSize.nothingToCombine, unconfirmed: false, full: 'export' }
          : outcome.kind === 'still-high'
            ? { message: text.pageSize.saveFullBudget, unconfirmed: false, full: 'export' }
            : { message: text.pageSize.compactedSaveAgain, unconfirmed: false, full: 'compacted' },
    );
  }
  const refreshSize = size.refresh;
  useEffect(() => {
    if (panel === 'about' || panel === 'source') void refreshSize();
  }, [panel, refreshSize]);
  const [selectionMode, setSelectionMode] = useState(false);
  useEffect(() => {
    if (!archived) return;
    setSelectionMode(false);
    setPanel((current) => (current === 'agents' || current === 'export' ? null : current));
  }, [archived]);
  const toggleSelectionMode = useRef<() => void>(() => {});
  const escapeAnnotation = useRef<() => boolean>(() => false);
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
    setChatActivation(undefined);
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
  const numbers = useMemo(() => threadNumbers(view.threads ?? []), [view.threads]);
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
      if (document.querySelector('dialog:modal')) return;
      const target = event.target;
      if (
        target instanceof Element &&
        !popover.current?.contains(target) &&
        !target.closest('[role="listbox"]')
      )
        closeRef.current(false);
    };
    document.addEventListener('pointerdown', outside, true);
    return () => document.removeEventListener('pointerdown', outside, true);
  }, [annotation]);
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
  const imageBinding = snapshot.binding?.files?.attachments;
  const imageScope = imageBinding?.scope?.();
  const imageDescriptors = JSON.stringify(view.attachments ?? []);
  const imageInput = useRef({ binding: imageBinding, descriptors: view.attachments ?? [] });
  imageInput.current = { binding: imageBinding, descriptors: view.attachments ?? [] };
  useEffect(() => {
    renderer.current?.images(imageInput.current);
  }, [imageBinding, imageScope, imageDescriptors]);
  const [state, setState] = useState<RenderState | 'loading'>('loading');
  // Author loading does not block discussion: sends use the captured quote.
  const discussionBlocked = archived || !!liveError || state === 'failed' || state === 'navigation';
  const canSelect = !!snapshot.binding?.discussion && !discussionBlocked && state === 'ready';
  activateChat.current = () => {
    if (
      archived ||
      (panel !== null && panel !== 'chat') ||
      [...document.querySelectorAll<HTMLElement>('[role="menu"], dialog[open]')].some(
        (node) => node.getClientRects().length > 0 && node.dataset.panel !== 'chat',
      )
    )
      return false;
    const insertMention = panel !== 'chat';
    setChatOpened(true);
    setPanel('chat');
    setChatActivation(
      !discussionBlocked && !recoveryRequired && !chatBusy.current
        ? { id: crypto.randomUUID(), insertMention }
        : undefined,
    );
    return true;
  };
  /** Whether there was anything to escape; an Escape with nothing to leave stays free for the
   * chapter panel to close. */
  escapeAnnotation.current = () => {
    if (panel === 'chat') {
      if (!chatBusy.current) setPanel(null);
    } else if (annotationRef.current) closeRef.current(true);
    else if (canSelect && selectionMode) setSelectionMode(false);
    else return false;
    return true;
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
        quickSwitcherOwner.current?.open ||
        event.defaultPrevented ||
        editing ||
        event.repeat ||
        event.isComposing ||
        event.keyCode === 229 ||
        (event.code !== 'KeyC' && event.key !== 'Escape' && event.key !== '@') ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        (event.shiftKey && event.key !== '@') ||
        (event.key !== '@' && !canSelect)
      )
        return;
      if (event.key === '@') {
        if (activateChat.current()) event.preventDefault();
        return;
      }
      if (event.key === 'Escape') {
        if (escapeAnnotation.current()) event.preventDefault();
        return;
      }
      event.preventDefault();
      if (!selectionMode && currentSelector.current && currentRectangle.current) annotate();
      else toggleSelectionMode.current();
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [canSelect, selectionMode]);
  const host = useRef<HTMLDivElement>(null);
  const { read: readingHint } = page.useSearch();
  const {
    readings,
    inlineSlots,
    reviewLayer,
    readingLayer,
    questionLayer,
    onQuestionAnchors,
    onReadingAnchors,
  } = usePageLayers({
    transport,
    snapshot,
    view,
    binding,
    renderer,
    host,
    state,
    discussionBlocked,
    statusCoordinator,
    connectionPresentation,
    readingHint: space ? readingHint : undefined,
  });
  const proposalLayer = useProposalLayer({
    slotPositions: inlineSlots.positions,
    snapshot,
    view,
    binding,
    discussionBlocked,
    recoveryRequired,
    connectionPresentation,
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
    if (endedError) {
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
      onQuestions: onQuestionAnchors,
      onReadings: onReadingAnchors,
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
        if (quickSwitcherOwner.current?.open) return;
        if (key === 'Escape') {
          if (!escapeAnnotation.current()) escapeChapter(router);
        } else if (key === '@') activateChat.current();
        else quickSwitcherOwner.current?.activate();
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
        else {
          renderer.current = handle;
          handle.images(imageInput.current);
        }
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
  }, [view.source, endedError, inlineSlots.onSlots, onQuestionAnchors, onReadingAnchors, router]);
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
                ordinal: numbers.get(`${thread.ref.writer}:${thread.threadId}`)!,
              },
            ]
          : [],
      ),
      annotation?.selector ?? undefined,
      numbers.size,
    );
  }, [state, view.threads, annotation, numbers]);
  // A modal drawer makes the page-level card inert, so an ended connection also speaks in the
  // drawer: the same sentence and action under its header. A reconnecting page shows nothing.
  const endedNotice = endedError && (
    <div className="drawer-recovery" role="status">
      <span className="drawer-recovery-state">
        <span aria-hidden="true">✗ </span>
        {text.blocked}
      </span>
      <EndedBody error={endedError} action={endedAction} compact />
      <EndedLink action={endedAction} />
    </div>
  );
  const notificationScope = JSON.stringify([
    space ?? '',
    snapshot.id,
    snapshot.binding?.discussion?.deviceId ?? '',
  ]);
  const notificationClassification = { questionThreads: questionLayer.notificationThreads };
  const chatScope = snapshot.binding?.discussion?.scope;
  const notifications = useReplyNotifications({
    pageKey: notificationScope,
    observer: replyObserver,
    sources: view,
    deviceId: snapshot.binding?.discussion?.deviceId,
    classification: notificationClassification,
    blocked: archived || !!endedError,
    chatReadingAllowed: panel === 'chat' && !quickSwitcher?.open && !recoveryRequired,
    chatScope: chatScope
      ? JSON.stringify([
          chatScope.spaceId,
          chatScope.pageId,
          chatScope.epoch,
          snapshot.binding?.discussion?.deviceId,
        ])
      : undefined,
    activate: (event) => {
      if (
        panel ||
        annotationRef.current?.thread ||
        annotationBusy.current ||
        statusBusy.current ||
        chatBusy.current
      )
        return false;
      if (event.destination === 'Chat') {
        if (annotationRef.current) closeRef.current(false);
        setChatOpened(true);
        setChatActivation(undefined);
        setPanel('chat');
      } else if (event.destination === 'Answer') {
        return questionLayer.activateNotification(event);
      } else if (event.destination === 'Proposal' && event.thread) {
        const target = proposalLayer.activateNotification(event.thread);
        if (target === 'comments') {
          setActiveThread(`${event.thread.writer}:${event.thread.id}`);
          setPanel('comments');
        }
        return !!target;
      } else if (event.thread) openAnchoredThread(event.thread);
      else return false;
      return true;
    },
  });
  const content = (
    <section className="page">
      <BrowserNotifications
        pageKey={notificationScope}
        events={notifications.events}
        suspended={
          notifications.activating ||
          panel !== null ||
          !!annotation?.thread ||
          notifications.modalCount > 0 ||
          recoveryRequired ||
          quickSwitcher?.open === true
        }
        isEligible={notifications.isEligible}
      />
      <ColabHeader
        remoteMenu={false}
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
            {questionLayer.ready && reviewLayer.ready && readingLayer.ready && (
              <NeedsYouHeader
                href={(item) =>
                  router.history.createHref(
                    router.buildLocation({
                      to: '/pages/$pageId',
                      params: { pageId: snapshot.id },
                      ...(item.kind === 'reading' ? { search: { read: item.id } } : {}),
                    }).publicHref,
                  )
                }
                items={pendingNeedsYou([
                  ...proposalLayer.navigation,
                  ...readingLayer.items
                    .filter((item) => inlineSlots.readingIds.has(item.id))
                    .map((item) => ({
                      ...item,
                      kind: 'reading' as const,
                      agent: readings.find((reading) => reading.id === item.id)?.agent ?? '',
                    })),
                  ...questionLayer.navigation,
                  ...reviewLayer.navigation,
                ])}
                onPick={(item) => {
                  if (item.kind === 'proposal') {
                    if (!proposalLayer.jump(item.id)) {
                      const ref = proposalLayer.reference(item.id);
                      if (ref) openThread(ref);
                    }
                  } else if (item.kind === 'reading') readingLayer.jump(item.id, true);
                  else if (item.kind === 'answer') questionLayer.jump(item.targetId ?? item.id);
                  else if (item.kind === 'review') reviewLayer.jump(item.id);
                }}
              />
            )}
            <span
              className={`chip page-sharing status ${state === 'ready' ? 'live' : ''}`}
              title={
                archived
                  ? text.archiveReadOnlyHint
                  : recoveryRequired
                    ? text.reconnecting
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
                    ? text.reconnecting
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
            chatUnread={notifications.chatUnread}
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
        notice={endedNotice}
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
          connectionOwner.report(new Error(managementChanged));
        }}
      />
      <BrowserDrawer
        notice={endedNotice}
        focusFallback={() => document.querySelector<HTMLButtonElement>('.tmt-ui-header button')}
        open={panel === 'about'}
        title={text.aboutPage}
        panel="about"
        onDismiss={() => setPanel(null)}
      >
        <PageAttribution
          originalAuthor={view.originalAuthor}
          publisherAgent={view.publisherAgent}
        />
        <PageSizeSection size={size} exportPage={() => setPanel('export')} />
        <p>{text.warning}</p>
      </BrowserDrawer>
      <div className="workspace">
        <div className="canvas">
          <div className="frame-host" ref={host} />
          {proposalLayer.inline}
          {reviewLayer.inline}
          {readingLayer.inline}
          {questionLayer.inline}
          {(state === 'navigation' || state === 'failed') && (
            <NoticeCard
              state="blocked"
              eyebrow={text.product}
              title={text.blocked}
              actions={<EndedLink action={endedAction} />}
            >
              {endedError ? (
                <EndedBody error={endedError} action={endedAction} />
              ) : (
                <>
                  <p>{state === 'navigation' ? text.navigation : text.failed}</p>
                  {state === 'failed' &&
                    new TextEncoder().encode(view.source).length > MAX_RENDER_SOURCE_BYTES && (
                      <p>{text.limit}</p>
                    )}
                </>
              )}
            </NoticeCard>
          )}
        </div>
      </div>
      {(canSelect || (state === 'ready' && !liveError)) && (
        <PageFloatingActions
          host={host.current}
          slots={inlineSlots.positions}
          enabled={selectionMode}
          discussionOpen={!!annotation}
          panel={panel}
          toggle={() => toggleSelectionMode.current()}
          chatUnread={notifications.chatUnread}
          chatButton={floatingChat}
          toggleChat={() => toggle('chat')}
          canSelect={canSelect}
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
                  connectionPresentation={connectionPresentation}
                  archived={archived}
                  thread={annotationThread}
                  number={
                    annotationThread
                      ? numbers.get(`${annotationThread.ref.writer}:${annotationThread.threadId}`)
                      : undefined
                  }
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
                        connectionPresentation={connectionPresentation}
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
        <BrowserDrawer
          notice={endedNotice}
          focusFallback={() => document.querySelector<HTMLButtonElement>('.tmt-ui-header button')}
          open={panel === 'files'}
          title={text.files}
          panel="files"
          onDismiss={() => setPanel(null)}
        >
          <FilesPanel
            descriptors={view.attachments ?? []}
            files={snapshot.binding.files}
            readOnly={archived}
            disabled={!!liveError || saving}
          />
        </BrowserDrawer>
      )}
      <BrowserDrawer
        notice={endedNotice}
        focusFallback={() => document.querySelector<HTMLButtonElement>('.tmt-ui-header button')}
        open={panel === 'source'}
        title={text.source}
        panel="source"
        width="wide"
        onDismiss={() => setPanel(null)}
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
          {/* Save problems take priority over routine usage in this one notice slot. */}
          {editError ? (
            <SaveNotice
              problem={editError}
              actions={
                editError.full === 'compact' ? (
                  <BrowserAction
                    type="button"
                    label={text.pageSize.compact}
                    variant="text"
                    disabled={size.compacting}
                    onActivate={() => void compactFromRefusal()}
                  />
                ) : editError.full === 'export' ? (
                  <BrowserAction
                    type="button"
                    label={text.pageSize.export}
                    variant="text"
                    onActivate={() => setPanel('export')}
                  />
                ) : editError.conflict ? (
                  <>
                    <BrowserAction
                      type="button"
                      label={text.saveCopyDraft}
                      variant="text"
                      onActivate={(event) => {
                        if (event.isTrusted) copyDraft();
                      }}
                    />
                    {copied && (
                      <span role="status">
                        {copied === 'copied' ? text.saveDraftCopied : text.saveDraftNotCopied}
                      </span>
                    )}
                  </>
                ) : undefined
              }
            />
          ) : (
            <PageSizeNotice size={size} exportPage={() => setPanel('export')} />
          )}
          {merged && !editError && (
            <div className="source-notice">
              <p role="status">{text.saveMerged}</p>
            </div>
          )}
          <textarea
            id="source-edit"
            ref={sourceArea}
            readOnly={archived || !snapshot.binding || saving || !!liveError}
            spellCheck={false}
            value={draft}
            onChange={(event) => {
              dirty.current = true;
              setMerged(false);
              setDraft(event.target.value);
            }}
          />
        </div>
      </BrowserDrawer>
      <BrowserDrawer
        notice={endedNotice}
        focusFallback={() => document.querySelector<HTMLButtonElement>('.tmt-ui-header button')}
        open={panel === 'comments'}
        title={text.comments}
        panel="comments"
        onDismiss={() => setPanel(null)}
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
          connectionPresentation={connectionPresentation}
          archived={archived}
          reopenProposal={proposalLayer.reopenProposal}
          creationRecipient={view.creationRecipient}
          hideHeader
          numbers={numbers}
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
      </BrowserDrawer>
      <BrowserDrawer
        notice={endedNotice}
        focusFallback={() => document.querySelector<HTMLButtonElement>('.tmt-ui-header button')}
        open={panel === 'agents'}
        title={text.agentStatus}
        panel="agents"
        onDismiss={() => setPanel(null)}
      >
        <AgentStatusPanel
          open={panel === 'agents'}
          binding={snapshot.binding?.ask}
          page={snapshot.id}
          admitted={!!snapshot.binding && !liveError}
        />
      </BrowserDrawer>
      <AttachmentDropPanel key={`chat-drop:${snapshot.id}`} target="chat" active={panel === 'chat'}>
        <BrowserDrawer
          notice={archived ? endedNotice : undefined}
          focusFallback={() =>
            floatingChat.current ??
            document.querySelector<HTMLButtonElement>('.tmt-ui-header button')
          }
          open={panel === 'chat'}
          title="Chat"
          panel="chat"
          caption={text.chatVisible}
          onDismiss={(reason) => {
            if (reason === 'close') setPanel(null);
            else escapeAnnotation.current();
          }}
        >
          {chatOpened && (
            <ChatPanel
              connectionPresentation={connectionPresentation}
              archived={archived}
              creationRecipient={view.creationRecipient}
              key={`chat:${snapshot.id}:${draftsReady}`}
              initialEdit={drafts.current.get(CHAT_DRAFT)}
              activation={
                panel === 'chat' &&
                draftsReady &&
                chatActivation &&
                !discussionBlocked &&
                !chatBusy.current
                  ? {
                      ...chatActivation,
                      consumed: () =>
                        setChatActivation((current) =>
                          current?.id === chatActivation.id ? undefined : current,
                        ),
                    }
                  : undefined
              }
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
        </BrowserDrawer>
      </AttachmentDropPanel>
    </section>
  );
  const creatorScope = JSON.stringify([
    space ?? '',
    snapshot.id,
    loaded.binding?.discussion?.deviceId ?? '',
  ]);
  return (
    <CreatorDefaultProvider key={creatorScope} scope={creatorScope}>
      <ReplyNotificationContext
        value={{
          ...notifications.context,
          blockSurface:
            notifications.modalCount ||
            annotation?.thread ||
            recoveryRequired ||
            quickSwitcher?.open
              ? 'covered'
              : panel === 'comments'
                ? 'comments'
                : panel
                  ? 'covered'
                  : 'page',
        }}
      >
        <QuickSwitcherModalPresence />
        <AgentPresentation value={agents}>{content}</AgentPresentation>
      </ReplyNotificationContext>
    </CreatorDefaultProvider>
  );
}
export function createAppRouter(transport: PageTransport, space?: string) {
  const history = space
    ? createBrowserHistory({
        parseLocation: () => {
          const fragment = new URLSearchParams(location.hash.slice(1));
          let path = fragment.get('space') === space ? (fragment.get('path') ?? '/') : '/blocked';
          if (!/^\/(?:all|pages\/[0-9a-f-]+|short\/[0-9a-f-]{8,36})?$/.test(path))
            path = '/blocked';
          const read = ownerReadingHint(new URL(location.href), space);
          const search = read ? `?read=${read}` : '';
          return {
            href: path + search,
            pathname: path,
            search,
            hash: '',
            state: { ...window.history.state, __TSR_index: window.history.state?.__TSR_index ?? 0 },
          };
        },
        createHref: (path) => ownerReadingHref(path, location.pathname, space),
      })
    : createHashHistory();
  const router = createRouter({
    routeTree: root.addChildren([home, allPages, page, shortPage, blocked]),
    history,
    context: { transport, space },
  });
  router.subscribe('onResolved', () => void buildWatch.check());
  router.subscribe('onRendered', () => finishReturn(router));
  return router;
}
declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAppRouter>;
  }
}
