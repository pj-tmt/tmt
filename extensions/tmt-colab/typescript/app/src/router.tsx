import { ShellPage } from './shell-page.js';
import { PageWorkBand } from './work-band-live.js';
import { usePageLinks } from './page-link-bar.js';
import { usePageLayers } from './page-layers.js';
import { AgentActivityProvider } from './components/agent-activity.js';
import { RemoteAttention, RemotePresenceProvider } from './remote-header.js';
import { PagePinNotice, PageProjects } from './page-projects.js';
import type { PagePinRequest, PagePinState } from './pins.js';
import './source-editor.css';
import { OwnPublicationTimeout } from './own-publication.js';
import { entryMount, entryOpenTarget, inShell, publicEntry } from './entry.js';
import {
  BrowserAction,
  BrowserCommand,
  BrowserIconAction,
  BrowserNotifications,
  BusyMark,
  HostedShell,
} from '@tmt/browser-ui/react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { shortPageId, validPagePrefix } from './short-links.js';
import {
  hintFragment,
  hintSearch,
  ownerPageHint,
  ownerReadingHref,
  openSearch,
  pageSearch,
  splitPageHint,
  type PageHint,
} from './reading-link.js';
import { ArrowUpRight, MessageCircle, MessageSquare, Moon, MousePointer2, Sun } from 'lucide-react';
import { moreButtonRect, PageHeaderActions, type PageHeaderAction } from './page-header-actions.js';
import { ColabHeader } from './colab-header.js';
import { PageAttribution } from './page-attribution.js';
import { PageLayoutSection } from './page-layout-section.js';
import { ErrorDetails, NoticeCard } from './notice-card.js';
import { openFailure } from './open-failure.js';
import {
  useContext,
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
  useParams,
  useRouterState,
  useRouter,
} from '@tanstack/react-router';
import type { PageView, PageSnapshot, PageTransport } from './transport.js';
import type { PageRouting } from './page-routing.js';
import { AnnotationInput } from './annotation-input.js';
import { useProposalLayer } from './proposal-layer.js';
import { rememberPages, SwitcherSourceStore } from './quick-switcher-source.js';
import { noteOpened } from './recently-opened.js';
import { QuickSwitcherHost, QuickSwitcherModalPresence } from './quick-switcher-dialog.js';
import { isListPath, keepListSearch, storedListSearch } from './list-history.js';
import { ColabShell, ShellHostSlot } from './colab-shell.js';
import { usePageEnvironment } from './page-environment.js';
import { ColabSettings } from './colab-settings.js';
import { useQuickSwitcher } from './quick-switcher-context.js';
import { NeedsYouHeader } from './needs-you-header.js';
import { shareView } from './share-view.js';
import { pendingNeedsYou } from './needs-you.js';
import { useReplyNotifications } from './use-reply-notifications.js';
import { ReplyNotificationObserver } from './reply-notifications.js';
import { ReplyNotificationContext } from './reply-notification-context.js';
import { ThreadPanel, ThreadWindow } from './thread-panel.js';
import { presentationOf } from './thread-status-presentation.js';
import { isStatusThread, openThreadCount } from './thread-status-view.js';
import type { QuoteSelector } from './thread-records.js';
import { HeaderSlot } from './header-slot.js';
import { Home, validateHomeSearch, type HomeSearch } from './home.js';
import { knownScope, readScope } from './project-scope.js';
import { ManagePage } from './manage-page.js';
import { mountRenderer, MAX_RENDER_SOURCE_BYTES } from './renderer.js';
import { framedLayout } from './page-layout.js';
import type { AnchorPosition, RenderState, SelectionRect } from './renderer.js';
import { text } from './strings.js';
import { getTheme, setThemeChoice, subscribeTheme } from './theme.js';
import type { ConnectionState, EndedAction } from './connection-model.js';
import { useConnection } from './use-connection.js';
import { terminalFailure } from './terminal-failure.js';
import { buildWatch } from './app-build.js';
import { SessionEvictedError } from './ask-remote.js';
import { SessionLimitError } from './session-limit-error.js';
import { SessionLimitBody } from './session-limit.js';
import { ExportPanel } from './export-panel.js';
import { BrowserDrawer } from '@tmt/browser-ui/react';
import { AgentStatusPanel } from './agent-status-panel.js';
import { AgentPresentation, useAgentPresentation } from './agent-directory.js';
import { ChatPanel } from './chat-panel.js';
import { AttachmentDropPanel } from './attachment-drop.js';
import { FilesPanel } from './files-panel.js';
import { CreatorDefaultProvider } from './creator-default.js';
import { CHAT_DRAFT, SavedDrafts, savedDrafts } from './saved-drafts.js';
import { isChatThread, threadNumbers } from './thread-records.js';
import { PageSizeNotice, PageSizeSection, usePageSize } from './page-size.js';
import { usePageEditor } from './use-page-editor.js';
import { usePagePanels } from './use-page-panels.js';
import { useAnnotations } from './use-annotations.js';
import { usePageRenderer } from './use-page-renderer.js';
import { SaveNotice } from './save-notice.js';

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
  selectPending,
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
  /** The button's place while the threads are unverified: fenced, not greyed. */
  selectPending: boolean;
}) {
  const element = useRef<HTMLDivElement>(null);
  const [placement, setPlacement] = useState<{
    right: number;
    bottom: number;
    visibility: 'visible' | 'hidden';
  }>({ right: 24, bottom: 24, visibility: 'hidden' });
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
        .map((footer) =>
          (footer.closest('.question-compact-block') ?? footer).getBoundingClientRect(),
        )
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
        {(canSelect || selectPending) && (
          <BrowserIconAction
            type="button"
            variant="primary"
            busy={selectPending}
            label={text.selectionMode}
            ariaKeyShortcuts={enabled ? 'c Escape' : 'c'}
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
  thread,
  arrivals,
  children,
}: {
  host: HTMLDivElement | null;
  rectangle: SelectionRect | null;
  inset: number;
  noticeVisible: boolean;
  toggleVisible: boolean;
  open(): void;
  /** The thread the window shows, so a different thread starts from its own place. */
  thread?: string;
  /** The records in the window's thread; an arrival after the first grows it upward, not downward. */
  arrivals: number;
  children?: React.ReactNode;
}) {
  const expanded = children !== undefined;
  const element = useRef<HTMLDivElement>(null);
  // The lower edge a thread window keeps once a record arrives in it, relative to its anchor, and
  // the height and arrivals seen by the last placement.
  const rise = useRef<number | null>(null);
  const placed = useRef({ height: 0, arrivals });
  useLayoutEffect(() => {
    rise.current = null;
    placed.current.arrivals = arrivals;
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only a different thread resets
  }, [thread]);
  useLayoutEffect(() => {
    if (arrivals > placed.current.arrivals && placed.current.arrivals > 0)
      rise.current ??= placed.current.height;
    if (arrivals === 0) rise.current = null;
    placed.current.arrivals = arrivals;
  }, [arrivals]);
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
      const anchoredTop = Math.max(
        inset + 4,
        Math.min(
          edge - placementHeight,
          beside || besideLeft ? top : below ? bottom + 6 : top - placementHeight - 6,
        ),
      );
      // A reply arriving in a thread grows the window upward: the composer keeps its viewport
      // position and the new message appears above it. The window scrolls inside once it is tall.
      const height = element.current?.getBoundingClientRect().height ?? 0;
      placed.current.height = height;
      const settledBottom = rise.current === null || above ? null : anchoredTop + rise.current;
      const windowTop =
        settledBottom === null ? anchoredTop : Math.max(inset + 4, settledBottom - height);
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
          : {
              top: windowTop,
              // The window never reaches below where its thread first settled: longer history scrolls inside.
              maxHeight: Math.max(
                0,
                settledBottom === null ? edge - windowTop : settledBottom - (inset + 4),
              ),
            }),
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

/** A terminal refusal as a sentence; its raw code stays as a small reference. */
function TerminalFailure({
  error,
  reference: showReference = true,
}: {
  error: Error;
  reference?: boolean;
}) {
  if (error.message === managementChanged) return <p>{managementChanged}</p>;
  const { sentence, reference, referenceLabel } = terminalFailure(error);
  return (
    <>
      <p>{sentence}</p>
      {showReference && reference && (
        <p>
          <small className="failure-reference">
            {text.failureCodeLabel}{' '}
            <code
              className="tmt-ui-code"
              data-failure-reference
              title={reference}
              data-own-publication={
                error instanceof OwnPublicationTimeout && error.diagnostic
                  ? JSON.stringify(error.diagnostic)
                  : undefined
              }
            >
              {referenceLabel}
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
          <BrowserCommand text={text.pairCommand} />
        </>
      )}
    </>
  );
}

function EndedLink({ action, primary = false }: { action: EndedAction; primary?: boolean }) {
  return (
    <>
      {action === 'reload' && (
        <BrowserAction
          type="button"
          variant={primary ? 'primary' : 'text'}
          label={text.reload}
          onActivate={() => location.reload()}
        />
      )}
      <Link className={ui.action} data-variant="text" to="/">
        {text.retry}
      </Link>
    </>
  );
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

type KnownPages = readonly { pageId: string; deleted: boolean }[];
type AppContext = {
  transport: PageTransport;
  space?: string;
  // The short-URL history resolves a prefix against the catalog the loaders last saw.
  rememberPages?: (pages: KnownPages) => void;
};
const pageOpenFailureTitle = (failure: { kind: 'retry' | 'large' }) =>
  failure.kind === 'retry' ? text.openFailed : text.openTooLarge;
/**
 * A page that could not open. A failure that reopening may cure is plain copy with a Retry that
 * runs the page's loader again (the router then shows the same opening frame as a first open), and
 * the raw token moves to a small reference; only a refusal keeps the red card (see `openFailure`).
 * Return to space is always there.
 */
function PageOpenFailure({
  failure,
}: {
  failure: Exclude<ReturnType<typeof openFailure>, { kind: 'refused' }>;
}) {
  const router = useRouter();
  const retry = failure.kind === 'retry';
  return (
    <NoticeCard
      state="inactive"
      stateLabel={text.openFailedLabel}
      eyebrow={text.product}
      title={pageOpenFailureTitle(failure)}
      testId="page-open-failure"
      actions={
        <>
          {retry && (
            <BrowserAction
              type="button"
              variant="primary"
              label={text.openRetry}
              onActivate={() => void router.invalidate()}
            />
          )}
          <Link className={ui.action} data-variant="text" to="/">
            {text.retry}
          </Link>
        </>
      }
    >
      <p>
        {retry ? (
          text.openFailedBody
        ) : (
          <>
            {text.openTooLargeBefore}
            <code className="tmt-ui-code">{text.openTooLargeCommand}</code>
            {text.openTooLargeAfter}
          </>
        )}
      </p>
      <p>
        <small className="failure-reference">
          {text.failureCodeLabel}{' '}
          <code className="tmt-ui-code" data-failure-reference>
            {retry ? failure.reference : text.openTooLargeLabel}
          </code>
        </small>
      </p>
    </NoticeCard>
  );
}
const root = createRootRouteWithContext<AppContext>()({
  component: Shell,
  errorComponent: function RootError({ error }) {
    const router = useRouter();
    const opening = useRouterState({ select: (state) => state.location.pathname }).startsWith(
      '/pages/',
    );
    const failure =
      error instanceof Error && (opening || error.message === text.reconnectFailed)
        ? openFailure(error)
        : undefined;
    const header = useRef<HTMLElement>(null);
    useLayoutEffect(() => {
      const title = header.current?.querySelector('h1');
      if (title) {
        title.tabIndex = -1;
        title.focus({ preventScroll: true });
      }
    }, []);
    const hostSlot = useContext(ShellHostSlot);
    // Inside the TMT web app the error view keeps the host's header slot, like every other view (#4373).
    const inHost = (view: ReactNode) =>
      hostSlot ? (
        <HostedShell slot={hostSlot} menu={undefined}>
          {view}
        </HostedShell>
      ) : (
        view
      );
    return inHost(
      <>
        <AppHeader
          title={failure && failure.kind !== 'refused' ? pageOpenFailureTitle(failure) : text.error}
          headerRef={header}
        />
        <main>
          {failure && failure.kind !== 'refused' ? (
            <PageOpenFailure failure={failure} />
          ) : (
            <NoticeCard
              state="blocked"
              eyebrow={text.product}
              title={text.error}
              actions={
                <>
                  <BrowserAction
                    type="button"
                    variant="text"
                    label={text.crashRetry}
                    onActivate={() => void router.invalidate()}
                  />
                  <Link className={ui.action} data-variant="text" to="/">
                    {text.retry}
                  </Link>
                </>
              }
            >
              {failure ? (
                <TerminalFailure error={error as Error} />
              ) : (
                <>
                  <p>{text.errorBody}</p>
                  <ErrorDetails error={error} />
                </>
              )}
            </NoticeCard>
          )}
        </main>
      </>,
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
async function loadSpaceHome({ context }: { context: AppContext }) {
  const home = await context.transport.spaceHome();
  context.rememberPages?.(
    home.pageIds ?? home.pages.map((page) => ({ pageId: page.id, deleted: false })),
  );
  // A quick switcher opened later shows this list at once and refreshes it.
  rememberPages(context.transport, context.space, home.pages, home.projects);
  return home;
}
const home = createRoute({
  getParentRoute: () => root,
  path: '/',
  validateSearch: validateHomeSearch,
  loader: async (options) => {
    const space = await loadSpaceHome(options);
    // Colab opens on the project last chosen here; a project that is gone opens All pages.
    if ((options.location.search as HomeSearch).scope === undefined) {
      const remembered = readScope(options.context.space);
      if (knownScope(remembered, space.projects) !== undefined)
        throw redirect({
          to: '/',
          search: { ...(options.location.search as HomeSearch), scope: remembered },
          replace: true,
        });
    }
    return space;
  },
  component: Home,
});
// The list was `/all` before it became the home; an old link or history entry still lands on it.
const allPages = createRoute({
  getParentRoute: () => root,
  path: '/all',
  validateSearch: validateHomeSearch,
  beforeLoad: ({ search }) => {
    throw redirect({ to: '/', search, replace: true });
  },
});
/**
 * Timings of opening a page, for whoever profiles it: local performance entries only, no report.
 * The frame appears after `pendingMs` and stays `pendingMinMs`; a fast open never shows it.
 */
const openMarks = { start: 'colab:page-open', frame: 'colab:page-frame', page: 'colab:page-shown' };
function markOpen(name: string, from?: string) {
  performance.mark(name);
  if (from) performance.measure(`${name} (from ${from})`, from, name);
}
/**
 * What the reader sees while a page loads: the real header with the title the list already held (the
 * space home loaded; else the neutral word) and one spinner centred in the page area. It mounts no
 * renderer, frame or page action and fetches nothing. The open has no deadline: the spinner stays
 * while the open progresses and gives way only to the router's error state when it really fails.
 */
function PageFrame() {
  const { transport, space } = root.useRouteContext();
  const pageId = useParams({ strict: false }).pageId;
  // The list the space home last loaded, which the quick switcher also shows at once.
  const listed = new SwitcherSourceStore(transport, space).initial();
  const title =
    listed.state === 'ready' ? listed.pages.find((row) => row.id === pageId)?.title : undefined;
  useEffect(() => markOpen(openMarks.frame, openMarks.start), []);
  return (
    <div data-page-frame="" aria-busy="true">
      <ColabHeader
        remoteMenu={false}
        title={title || text.loading}
        remote={{ deviceName: transport.backendName }}
        home={(brand, name) => (
          <Link to="/" aria-label={name} title={name}>
            {brand}
          </Link>
        )}
      />
      <div className="page-opening">
        <span role="status" aria-label={text.openingPage} className="page-opening-mark">
          <BusyMark />
        </span>
      </div>
    </div>
  );
}
const page = createRoute({
  getParentRoute: () => root,
  path: '/pages/$pageId',
  validateSearch: (search: Record<string, unknown>) => ({
    ...pageSearch(search),
    ...openSearch(search),
  }),
  beforeLoad: () => markOpen(openMarks.start),
  loader: ({ context, params, abortController }) =>
    context.transport.page(params.pageId, abortController.signal),
  // The router keeps the old screen until the load resolves; a slow one gets this frame instead.
  pendingComponent: PageFrame,
  pendingMs: 150,
  pendingMinMs: 300,
  onEnter: () => markOpen(openMarks.page, openMarks.start),
  // A loaded page owns a live connection that closes when the reader leaves it. The router must not
  // keep the match, or entering the page again shows that closed snapshot as Live while it reloads.
  gcTime: 0,
  // Staying on the page (a thread selection rewrites only the fragment) never reloads it: a reload
  // would replace the binding, closing the writer under any send in flight.
  staleTime: Infinity,
  component: Page,
});

const THREAD_FRAGMENT = /^#?t=[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const shortPage = createRoute({
  getParentRoute: () => root,
  path: '/short/$prefix',
  // A short link keeps the page's open chapter panel (`?open=`) as well as its one hint.
  validateSearch: (search: Record<string, unknown>) => ({
    ...pageSearch(search),
    ...openSearch(search),
  }),
  loader: async ({ context, params, location }) => {
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
      throw redirect({
        to: '/pages/$pageId',
        params: { pageId: pages[0].id },
        search: { ...pageSearch(location.search), ...openSearch(location.search) },
        // Inside the app the address is real: a thread the short link names rides to the page.
        ...(THREAD_FRAGMENT.test(location.hash) ? { hash: location.hash.replace(/^#/, '') } : {}),
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

const colabSettings = createRoute({
  getParentRoute: () => root,
  path: '/settings',
  component: () => <ColabSettings />,
});
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
  headingSuffix,
}: {
  linked?: boolean;
  title?: string;
  remote?: { deviceName?: string };
  headerRef?: Ref<HTMLElement>;
  headingSuffix?: ReactNode;
}) {
  return (
    <ColabHeader
      title={title}
      headerRef={headerRef}
      remote={remote}
      headingSuffix={headingSuffix}
      home={
        linked
          ? (brand, name) => (
              <Link to="/" aria-label={name} title={name}>
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
  const isHome = pathname === '/';
  // The home list portals its project control into the header, where it keeps the list's data.
  const [headerSlot, setHeaderSlot] = useState<HTMLElement | null>(null);
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
        <ColabShell transport={transport} space={space}>
          {!isPage && (
            <AppHeader
              remote={isHome ? { deviceName: transport.backendName } : undefined}
              headingSuffix={
                isHome ? <span ref={setHeaderSlot} className="header-project-slot" /> : undefined
              }
              title={
                pathname.startsWith('/short/')
                  ? 'Open a page'
                  : pathname === '/'
                    ? text.pages
                    : pathname === '/settings'
                      ? text.colabSettings.title
                      : text.error
              }
            />
          )}
          <main className={isPage ? 'page-main' : undefined}>
            <HeaderSlot.Provider value={isHome ? headerSlot : null}>
              <Outlet />
            </HeaderSlot.Provider>
          </main>
          {!isPage && (
            <footer>{location.pathname.startsWith('/r/') ? text.mountedNote : text.adapter}</footer>
          )}
        </ColabShell>
      </QuickSwitcherHost>
    </RemotePresenceProvider>
  );
}
/** The page route: what only a route can know (its loader data, its search, where the reader came
 * from and the router), handed to the page as props and a `PageRouting`. */
function Page() {
  const router = useRouter();
  const { transport, space } = root.useRouteContext();
  const loaded = page.useLoaderData();
  const search = page.useSearch();
  // The list the reader came from, when it was scoped to a project: history state, not the address.
  const fromScope = useRouterState({
    select: (state) => (state.location.state as { scope?: unknown }).scope,
  });
  const scope = typeof fromScope === 'string' ? fromScope : undefined;
  const routing = useMemo(() => routePageRouting(router), [router]);
  return (
    <OpenPage
      transport={transport}
      space={space}
      loaded={loaded}
      search={search}
      scope={scope}
      routing={routing}
    />
  );
}

/** The page route's own routing: links go to page routes of this router, and the brand and the
 * project crumb link back to the space list. */
function routePageRouting(router: ReturnType<typeof useRouter>): PageRouting {
  return {
    open(route, anchor) {
      void router.navigate({ to: route, search: anchor ? { anchor } : {} });
    },
    pageHref: (pageId, hint) =>
      router.history.createHref(
        router.buildLocation({ to: '/pages/$pageId', params: { pageId }, search: hint }).publicHref,
      ),
    home: (brand, name) => (
      <Link to="/" aria-label={name} title={name}>
        {brand}
      </Link>
    ),
    scopeCrumb: (scope) => (
      <Link to="/" search={{ scope }} className="colab-scope-crumb" title={scope}>
        {scope}
      </Link>
    ),
  };
}

function OpenPage({
  transport,
  space,
  loaded,
  search,
  scope,
  routing,
}: {
  transport: PageTransport;
  space: string | undefined;
  loaded: PageSnapshot;
  /** The reading, element or task a link or a shell item opened the page at. */
  search: { read?: string; anchor?: string; task?: string };
  scope: string | undefined;
  routing: PageRouting;
}) {
  const environment = usePageEnvironment();
  // The renderer's thread handler outlives a render, so it reads the environment through a ref.
  const environmentRef = useRef(environment);
  environmentRef.current = environment;
  const quickSwitcher = useQuickSwitcher();
  const quickSwitcherOwner = useRef(quickSwitcher);
  quickSwitcherOwner.current = quickSwitcher;
  // A reopened binding replaces the loaded snapshot until the route loads a newer one.
  const [reopened, setReopened] = useState<{
    from: typeof loaded;
    next: typeof loaded;
  } | null>(null);
  const snapshot = reopened?.from === loaded ? reopened.next : loaded;
  const replyObserver = useMemo(() => new ReplyNotificationObserver(snapshot), [snapshot]);
  const toolbar = useRef<HTMLElement>(null);
  const {
    reset: resetPageLinks,
    request: requestPageLink,
    bar: pageLinkBar,
  } = usePageLinks({
    base: () => (publicEntry() ? entryMount().href : location.href),
    space,
    inset: () => toolbar.current?.getBoundingClientRect().bottom ?? 0,
    navigate: routing.open,
  });
  const needsYouMenuFocus = useRef(false);
  const panels = usePagePanels(snapshot.id, snapshot.binding);
  // Handlers the renderer and the keys keep across renders reach the panels through a ref.
  const panelsRef = useRef(panels);
  panelsRef.current = panels;
  const {
    panel,
    chatOpened,
    chatActivation,
    activeThread,
    toggle,
    show,
    close,
    closeIf,
    clear,
    showThread,
    openChat,
    settleChatActivation,
    setActiveThread,
  } = panels;
  const floatingChat = useRef<HTMLButtonElement>(null);
  const activateChat = useRef<() => boolean>(() => false);

  // The page's own projects: the picker opens from its chips and from Projects… in the More menu.
  const [projects, setProjects] = useState<{ request: number; from?: DOMRect }>({ request: 0 });
  const openProjects = (from?: DOMRect) =>
    setProjects((previous) => ({ request: previous.request + 1, from }));
  const [pin, setPin] = useState<PagePinState>();
  const [pinRequest, setPinRequest] = useState<PagePinRequest>();
  const requestPin = (action: PagePinRequest['action']) =>
    setPinRequest((previous) => ({ id: (previous?.id ?? 0) + 1, action }));
  const [view, setView] = useState<PageView>({
    discussionPending: snapshot.discussionPending,
    pageType: snapshot.pageType,
    shell: snapshot.shell,
    work: snapshot.work,
    archived: snapshot.archived,
    source: snapshot.source,
    title: snapshot.title,
    originalAuthor: snapshot.originalAuthor,
    layout: snapshot.layout,
    layoutReason: snapshot.layoutReason,
    publisherAgent: snapshot.publisherAgent,
    ownData: snapshot.ownData ?? false,
    own: snapshot.own,
    asks: snapshot.asks,
    threads: snapshot.threads,
  });
  // The document shows before its threads and asks are verified; until they follow, nothing that
  // reads them speaks (#3894): no count, no band, no Needs you, and the discussion stays shut.
  const discussionPending = !!view.discussionPending;
  useEffect(() => {
    return environment.setTitle(`${view.title || snapshot.title} · ${text.product}`);
  }, [environment, view.title, snapshot.title]);
  // This device remembers which pages it opened, for a quick switcher to offer first; local only.
  useEffect(() => {
    void noteOpened(space, snapshot.id);
  }, [space, snapshot.id]);
  const latest = useRef(view);
  const size = usePageSize(snapshot.binding);
  const {
    draft,
    changed: draftChanged,
    saving,
    error: editError,
    merged,
    copied,
    sourceArea,
    saveSettled,
    adoptSnapshot,
    adoptUpdate,
    save,
    copyDraft,
    compactFromRefusal,
    type: typeSource,
  } = usePageEditor({ snapshot, latest, size });
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
  const endedPresentation = endedError ? terminalFailure(endedError) : undefined;
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
    adoptSnapshot(snapshot);
    setView({
      discussionPending: snapshot.discussionPending,
      pageType: snapshot.pageType,
      shell: snapshot.shell,
      work: snapshot.work,
      archived: snapshot.archived,
      source: snapshot.source,
      title: snapshot.title,
      originalAuthor: snapshot.originalAuthor,
      layout: snapshot.layout,
      layoutReason: snapshot.layoutReason,
      publisherAgent: snapshot.publisherAgent,
      ownData: snapshot.ownData ?? false,
      own: snapshot.own,
      asks: snapshot.asks,
      threads: snapshot.threads,
    });
    connectionOwner.reset();
    let firstView = true;
    const unsubscribe = snapshot.binding?.subscribe(
      (value) => {
        // Every update arrives as a fresh copy of the whole page's state; what did not change keeps
        // the identity it had, so only the sections keyed on what changed run again. The view
        // itself is always a new object: a publication with the same data still renders the page,
        // because the binding (its Ask facade, its files) can change without the data changing.
        const next = {
          ...shareView(latest.current, { ...value, ownData: value.ownData ?? false }),
        };
        if (firstView) {
          firstView = false;
          replyObserver.baselineFirstView(next);
        }
        latest.current = next;
        setView(next);
        adoptUpdate(value.source);
      },
      (error) => connectionOwner.report(error),
    );
    return () => {
      unsubscribe?.();
    };
  }, [snapshot, loaded, transport, connectionOwner, replyObserver, adoptSnapshot, adoptUpdate]);
  const refreshSize = size.refresh;
  useEffect(() => {
    if (panel === 'about' || panel === 'source') void refreshSize();
  }, [panel, refreshSize]);
  const toggleSelectionMode = useRef<() => void>(() => {});
  const escapeAnnotation = useRef<() => void>(() => {});
  const chatBusy = useRef(false);
  const binding = useRef(snapshot.binding);
  const renderer = useRef<Awaited<ReturnType<typeof mountRenderer>> | null>(null);
  const host = useRef<HTMLDivElement>(null);
  binding.current = snapshot.binding;
  const {
    selectionMode,
    setSelectionMode,
    selector,
    currentSelector,
    rectangle,
    currentRectangle,
    annotation,
    annotationRef,
    popover,
    drafts,
    draftsReady,
    draftsUnsaved,
    keepDraft,
    annotationDraft,
    annotationBusy,
    statusBusy,
    changingStatus,
    setChangingStatus,
    annotate,
    annotationKey,
    closeAnnotation,
    closeRef,
    onSelection,
    openThread,
    continueAnnotation,
    openAnchoredThread,
  } = useAnnotations({
    snapshot,
    transport,
    binding,
    latest,
    renderer,
    host,
    panels: {
      clear,
      showThread,
      setActiveThread,
    },
  });
  useEffect(() => {
    if (!archived) return;
    setSelectionMode(false);
    closeIf('agents', 'export');
  }, [archived, closeIf, setSelectionMode]);
  const annotationThread = annotation?.thread
    ? view.threads?.find(
        (thread) =>
          thread.ref.writer === annotation.thread!.writer &&
          thread.ref.id === annotation.thread!.id,
      )
    : undefined;
  const numbers = useMemo(() => threadNumbers(view.threads ?? []), [view.threads]);
  const openThreads = discussionPending ? undefined : openThreadCount(view.threads ?? []);
  const fileCount = view.attachments?.length ?? 0;
  const unseenThreads = (view.threadPresentations ?? []).some((value) => value.status.unseen);
  const statusCoordinator =
    archived || liveError?.message === managementChanged ? undefined : snapshot.binding?.status;
  const [state, setState] = useState<RenderState | 'loading'>('loading');
  const accessLabel = archived
    ? text.archiveReadOnly
    : recoveryRequired
      ? text.reconnecting
      : state === 'ready'
        ? text.live
        : state === 'loading'
          ? text.loading
          : text.blocked;
  const deepThread = useRef<string | null>(null);
  useEffect(() => {
    const fragment = environment.threadFragment();
    if (state !== 'ready' || !fragment) return;
    const target = fragment.read();
    if (!target || deepThread.current === `${snapshot.id}:${target}`) return;
    const thread = view.threads?.find((value) => value.threadId === target && !value.deleted);
    if (!thread) return;
    deepThread.current = `${snapshot.id}:${target}`;
    showThread(`${thread.ref.writer}:${thread.ref.id}`);
    renderer.current?.scrollAnchor(`${thread.ref.writer}:${thread.ref.id}`);
  }, [environment, showThread, state, snapshot.id, view.threads]);

  // Author loading does not block discussion: sends use the captured quote.
  const discussionBlocked =
    archived || !!liveError || state === 'failed' || state === 'navigation' || discussionPending;
  const selectable =
    view.pageType !== 'shell' && !!snapshot.binding?.discussion && state === 'ready';
  const canSelect = selectable && !discussionBlocked;
  // Selecting is offered once the threads are verified; until then its button holds its place.
  const selectPending = selectable && discussionPending && !archived && !liveError;
  activateChat.current = () => {
    if (archived || (panel !== null && panel !== 'chat') || environment.overlayOpen('chat'))
      return false;
    const insertMention = panel !== 'chat';
    openChat(
      !discussionBlocked && !recoveryRequired && !chatBusy.current
        ? { id: crypto.randomUUID(), insertMention }
        : undefined,
    );
    return true;
  };
  escapeAnnotation.current = () => {
    if (panel === 'chat') {
      if (!chatBusy.current) close();
    } else if (annotationRef.current) closeRef.current(true);
    else if (canSelect) setSelectionMode(false);
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
      event.preventDefault();
      if (event.key === 'Escape') escapeAnnotation.current();
      else if (!selectionMode && currentSelector.current && currentRectangle.current) annotate();
      else toggleSelectionMode.current();
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [canSelect, selectionMode, annotate, currentSelector, currentRectangle]);
  const { read: readingHint, anchor, task } = search;
  const {
    readings,
    inlineSlots,
    reviewLayer,
    readingLayer,
    questionLayer,
    onQuestionAnchors,
    onReadingAnchors,
  } = usePageLayers({
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
  const { resolved, anchorsChecked, pageScheme, missingAnchor } = usePageRenderer({
    snapshot,
    view,
    endedError,
    host,
    renderer,
    setState,
    toolbar,
    anchor,
    layers: {
      onSlots: inlineSlots.onSlots,
      onQuestions: onQuestionAnchors,
      onReadings: onReadingAnchors,
    },
    resetPageLinks,
    requestPageLink,
    handlers: {
      onSelection,
      onAnnotate: annotate,
      onToggleSelectionMode: () => toggleSelectionMode.current(),
      onKey: (key) => {
        if (quickSwitcherOwner.current?.open) return;
        if (key === 'Escape') escapeAnnotation.current();
        else if (key === '@') activateChat.current();
        else quickSwitcherOwner.current?.activate();
      },
      onOpenThread: (id) => {
        const thread = latest.current.threads?.find(
          (value) => `${value.ref.writer}:${value.threadId}` === id && !value.deleted,
        );
        if (thread) openAnchoredThread(thread.ref);
      },
    },
  });
  // Resolving hides the margin marker; Reopen restores it. Resolved history stays in Comments.
  // A reply or a status change elsewhere leaves this list as it was, so the document is not told
  // to highlight again.
  const highlights = useRef<readonly { id: string; selector: QuoteSelector; ordinal: number }[]>(
    [],
  );
  highlights.current = shareView(
    highlights.current,
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
  );
  const anchoredThreads = highlights.current,
    threadTotal = numbers.size;
  useEffect(() => {
    if (state !== 'ready') return;
    renderer.current?.highlight(
      anchoredThreads as { id: string; selector: QuoteSelector; ordinal: number }[],
      annotation?.selector ?? undefined,
      threadTotal,
    );
  }, [state, anchoredThreads, annotation, threadTotal]);
  // A modal drawer makes the page-level card inert, so an ended connection also speaks in the
  // drawer: the same sentence and action under its header. A reconnecting page shows nothing.
  const endedNotice = endedError && (
    <div className="drawer-recovery" role="status" data-tone={endedPresentation?.state}>
      <span className="drawer-recovery-state">
        <span aria-hidden="true">{endedPresentation?.state === 'inactive' ? '○ ' : '✗ '}</span>
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
        openChat();
      } else if (event.destination === 'Answer') {
        return questionLayer.activateNotification(event);
      } else if (event.destination === 'Proposal' && event.thread) {
        const target = proposalLayer.activateNotification(event.thread);
        if (target === 'comments') {
          showThread(`${event.thread.writer}:${event.thread.id}`);
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
        remoteChip={false}
        headingSuffix={
          <PageProjects
            transport={transport}
            space={space}
            pageId={snapshot.id}
            title={view.title || snapshot.title}
            request={projects.request}
            from={projects.from}
            open={() => openProjects()}
            pinRequest={pinRequest}
            onPinState={setPin}
          />
        }
        crumb={scope === undefined ? undefined : routing.scopeCrumb(scope)}
        headerRef={toolbar}
        title={view.title || snapshot.title || text.unknownPageTitle}
        remote={{ deviceName: transport.backendName }}
        caption={
          view.originalAuthor === undefined ? undefined : text.headerByAuthor(view.originalAuthor)
        }
        home={routing.home}
        status={
          <div className="page-header-meta">
            {!discussionPending &&
              questionLayer.ready &&
              reviewLayer.ready &&
              readingLayer.ready && (
                <NeedsYouHeader
                  menuFocus={needsYouMenuFocus}
                  href={(item) =>
                    routing.pageHref(
                      snapshot.id,
                      item.kind === 'reading' ? { read: item.id } : undefined,
                    )
                  }
                  items={pendingNeedsYou([
                    ...proposalLayer.navigation,
                    ...readingLayer.items
                      .filter((item) => inlineSlots.readingIds.has(item.id))
                      .map((item) => ({
                        ...item,
                        kind: 'reading' as const,
                        agents: [
                          readings.find((reading) => reading.id === item.id)?.agent ?? '',
                        ].filter(Boolean),
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
            {/* The connection state is not shown (a notice below the header carries trouble); its words
                stay in the document for assistive technology and as the page's readiness signal. */}
            <span
              className={`status page-connection tmt-ui-visually-hidden ${state === 'ready' ? 'live' : ''}`}
            >
              {archived ? '○ ' : `${text[snapshot.sharing]} · `}
              <span className="status-label">{accessLabel}</span>
            </span>
            <RemoteAttention />
            {archived ? (
              <span
                className="page-attention"
                data-attention="archived"
                title={text.archiveReadOnlyHint}
              >
                <span aria-hidden="true">○</span>
                <span className="page-attention-word">{text.archived}</span>
              </span>
            ) : (
              snapshot.sharing !== 'private' && (
                <span className="page-attention" data-attention="shared">
                  <span aria-hidden="true">◉</span>
                  <span className="page-attention-word">{text.shared}</span>
                </span>
              )
            )}
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
            access={`${archived ? '○ ' : `${text[snapshot.sharing]} · `}${accessLabel}`}
            canExport={!!snapshot.binding && !liveError && !archived}
            canManage={!!transport.management}
            canFiles={!!snapshot.binding?.files}
            canProjects={!!transport.management}
            pin={pin}
            onSelect={(action) => {
              if (action === 'projects') openProjects(moreButtonRect());
              else if (action === 'pin') requestPin('toggle');
              else toggle(action);
            }}
          />
        }
      />
      <PagePinNotice failure={pin?.failure} close={() => requestPin('dismiss')} />
      <p role="status" className="archive-announcement tmt-ui-visually-hidden">
        {archiveAnnouncement}
      </p>
      <ExportPanel
        key={`export:${snapshot.id}`}
        binding={snapshot.binding}
        blocked={!!liveError}
        drawer
        notice={endedNotice}
        opened={panel === 'export'}
        changeOpen={(open) => (open ? show('export') : close())}
      />
      <ManagePage
        pageId={snapshot.id}
        title={view.title || snapshot.title}
        open={panel === 'manage'}
        close={() => close()}
        changed={(operation) => {
          // Archive commits invalidate sync; verified read catch-up keeps this draft owner mounted.
          if (operation === 'page.archive') return;
          snapshot.binding?.close();
          connectionOwner.report(new Error(managementChanged));
        }}
      />
      <BrowserDrawer
        notice={endedNotice}
        focusFallback={environment.focusFallback}
        open={panel === 'about'}
        title={text.aboutPage}
        panel="about"
        onDismiss={() => close()}
      >
        <PageAttribution
          originalAuthor={view.originalAuthor}
          publisherAgent={view.publisherAgent}
        />
        <PageLayoutSection layout={view.layout} reason={view.layoutReason} />
        <PageSizeSection size={size} exportPage={() => show('export')} />
        <p>{text.warning}</p>
      </BrowserDrawer>
      <div className="workspace">
        {pageLinkBar}
        <PageWorkBand
          work={view.work}
          binding={snapshot.binding}
          transport={transport}
          space={space}
          pageId={snapshot.id}
          focusTask={task}
          pageHref={routing.pageHref}
        />
        <div className="canvas" data-layout={framedLayout(view.layout) ? view.layout : undefined}>
          {missingAnchor && (
            <p className="page-anchor-missing" role="status">
              {text.anchorMissing}
            </p>
          )}
          <div className="frame-host" ref={host} />
          {!discussionPending && (
            <>
              {/* The proposal and question cards sit on the author's page, so they take the scheme it
                  paints; the review and reading bands sit on the app's paper and keep the app's. */}
              <div className="page-cards" data-theme={pageScheme}>
                {proposalLayer.inline}
              </div>
              {reviewLayer.inline}
              {readingLayer.inline}
              <div className="page-cards" data-theme={pageScheme}>
                {questionLayer.inline}
              </div>
            </>
          )}
          {(state === 'navigation' || state === 'failed') && (
            <NoticeCard
              state={endedPresentation?.state ?? 'blocked'}
              stateLabel={endedPresentation?.state === 'inactive' ? text.notConnected : undefined}
              eyebrow={text.product}
              title={text.blocked}
              actions={
                <EndedLink action={endedAction} primary={endedPresentation?.state === 'inactive'} />
              }
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
          selectPending={selectPending}
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
            thread={annotationThread?.threadId}
            arrivals={
              annotationThread
                ? annotationThread.comments.length +
                  (view.asks ?? []).filter(
                    (ask) => ask.thread === annotationThread.threadId && ask.reply !== undefined,
                  ).length
                : 0
            }
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
          focusFallback={environment.focusFallback}
          open={panel === 'files'}
          title={text.files}
          panel="files"
          onDismiss={() => close()}
        >
          <FilesPanel
            descriptors={view.attachments ?? []}
            files={snapshot.binding.files}
            readOnly={archived}
            disabled={!!liveError}
            afterSave={() => saveSettled.current ?? Promise.resolve(true)}
          />
        </BrowserDrawer>
      )}
      <BrowserDrawer
        notice={endedNotice}
        focusFallback={environment.focusFallback}
        open={panel === 'source'}
        title={text.source}
        panel="source"
        width="wide"
        onDismiss={() => close()}
      >
        <div className="source">
          <span>
            <label htmlFor="source-edit">{text.source}</label>
            {snapshot.binding && !archived && (
              <BrowserAction
                type="button"
                variant="text"
                label={text.save}
                busy={saving}
                busyMark={<BusyMark />}
                disabled={!!liveError || !draftChanged}
                onActivate={() => void save()}
              />
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
                    onActivate={() => show('export')}
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
            <PageSizeNotice size={size} exportPage={() => show('export')} />
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
            onChange={(event) => typeSource(event.target.value)}
          />
        </div>
      </BrowserDrawer>
      <BrowserDrawer
        notice={endedNotice}
        focusFallback={environment.focusFallback}
        open={panel === 'comments'}
        title={text.comments}
        panel="comments"
        onDismiss={() => close()}
      >
        {discussionPending && !liveError && (
          <p role="status" className="discussion-pending">
            {text.discussionPending}
          </p>
        )}
        <SavedDrafts
          archived={archived}
          drafts={discussionPending ? [] : savedDrafts(drafts.current, view.threads)}
          discard={(key) => keepDraft(key, null)}
        />
        {draftsUnsaved && (
          <p role="status" className="annotation-hint saved-drafts-notice">
            {text.draftsNotSaved}
          </p>
        )}
        {!discussionPending && (
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
        )}
      </BrowserDrawer>
      <BrowserDrawer
        notice={endedNotice}
        focusFallback={environment.focusFallback}
        open={panel === 'agents'}
        title={text.agentStatus}
        panel="agents"
        onDismiss={() => close()}
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
          focusFallback={() => floatingChat.current ?? environment.focusFallback()}
          open={panel === 'chat'}
          title="Chat"
          panel="chat"
          caption={text.chatVisible}
          onDismiss={(reason) => {
            if (reason === 'close') close();
            else escapeAnnotation.current();
          }}
        >
          {chatOpened && discussionPending && !liveError && (
            <p role="status" className="discussion-pending">
              {text.discussionPending}
            </p>
          )}
          {chatOpened && !discussionPending && (
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
                      consumed: () => settleChatActivation(chatActivation.id),
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
              close={() => close()}
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
        <AgentPresentation value={agents}>
          <AgentActivityProvider
            observation={liveError || archived ? undefined : view.agentActivity}
            messages={liveError || archived ? undefined : view.messageStates}
          >
            {view.pageType === 'shell' ? (
              <section className="page">
                <AppHeader
                  title={view.title || snapshot.title}
                  remote={{ deviceName: transport.backendName }}
                />
                <main className="canvas">
                  {(view.shell || !endedError) && <ShellPage shell={view.shell} />}
                  {endedError && <EndedBody error={endedError} action={endedAction} />}
                </main>
              </section>
            ) : (
              content
            )}
          </AgentActivityProvider>
        </AgentPresentation>
      </ReplyNotificationContext>
    </CreatorDefaultProvider>
  );
}
/** The `space` an owner link named, kept by `shellLocation` until the router knows its space. */
let linkSpace: string | undefined;
/**
 * Inside the TMT app Colab's routes are paths under `/colab/`. Links printed or saved before keep
 * working: the preview's `#/pages/<id>` and an owner fragment's `path=` become the same path, with
 * its reading, anchor or task hint as search keys. The module runs it before anything reads the location
 * (the paired bootstrap keeps only a canonical fragment).
 */
export function shellLocation() {
  const hash = location.hash.slice(1);
  const fragment = new URLSearchParams(hash);
  // An owner link names its space; the router checks it once it knows this space.
  linkSpace = fragment.get('space') ?? undefined;
  const short = /^\/colab\/p\/([A-Za-z0-9_-]{4,64})$/.exec(location.pathname)?.[1];
  let path: string | undefined;
  let keep = '';
  if (hash.startsWith('/')) path = hash;
  else if (fragment.get('path')) path = fragment.get('path')!;
  else if (short) {
    // The short link `/p/<id>` lands here; a thread (`#t=`) stays a fragment for the page.
    path = `/short/${short}`;
    if (fragment.has('t')) keep = location.hash;
  }
  if (!path) return;
  const keys = new URLSearchParams(location.search);
  for (const key of ['read', 'anchor', 'task', 'open'])
    if (fragment.has(key)) keys.set(key, fragment.get(key)!);
  const search = keys.size ? `?${keys}` : '';
  if (/^\/(?:all|settings|pages\/[0-9a-f-]+|short\/[A-Za-z0-9_-]{4,64})?$/.test(path))
    history.replaceState(history.state, '', `/colab${path}${search}${keep}`);
}
export function createAppRouter(
  transport: PageTransport,
  space?: string,
  pageIds: readonly { pageId: string; deleted: boolean }[] = [],
) {
  let knownPages = pageIds;
  // An owner link to another space opens the blocked view, never a page of this one.
  if (inShell() && space && linkSpace !== undefined && linkSpace !== space)
    window.history.replaceState(window.history.state, '', '/colab/blocked');
  linkSpace = undefined;
  const history = inShell()
    ? createBrowserHistory()
    : space
      ? createBrowserHistory({
          parseLocation: () => {
            const fragment = new URLSearchParams(location.hash.slice(1));
            const prefix = location.pathname.startsWith('/p/') ? location.pathname.slice(3) : '';
            let path: string;
            let hint: PageHint | undefined;
            if (publicEntry()) {
              try {
                const target = entryOpenTarget();
                const matches = knownPages.filter((row) => row.pageId.startsWith(prefix));
                path = !prefix
                  ? '/'
                  : matches.length === 1 && !matches[0].deleted
                    ? `/pages/${matches[0].pageId}`
                    : `/short/${prefix}`;
                // A thread names a place inside the page; a reading or shell anchor is a page hint.
                if (target && !('thread' in target) && path !== '/') hint = target;
                if (hint && !('read' in hint) && path === '/') hint = undefined;
              } catch {
                path = '/blocked';
              }
            } else {
              path = fragment.get('space') === space ? (fragment.get('path') ?? '/') : '/blocked';
              hint = ownerPageHint(new URL(location.href), space);
            }
            if (!/^\/(?:all|pages\/[0-9a-f-]+|short\/[0-9a-f-]{8,36})?$/.test(path))
              path = '/blocked';
            const search = hint
              ? hintSearch(hint)
              : path === '/'
                ? storedListSearch(window.history.state)
                : '';
            return {
              href: path + search,
              pathname: path,
              search,
              hash: '',
              state: {
                ...window.history.state,
                __TSR_index: window.history.state?.__TSR_index ?? 0,
              },
            };
          },
          createHref: (routerPath) => {
            // The list's search is never part of the address (see `keepListSearch`).
            const path = isListPath(routerPath) ? '/' : routerPath;
            if (!publicEntry()) return ownerReadingHref(path, location.pathname, space);
            const { route, hint } = splitPageHint(path);
            const id = route.replace(/^\/(?:pages|short)\//, '');
            if (route === '/' || !validPagePrefix(id)) return '/colab/';
            const current = location.pathname.startsWith('/p/') ? location.pathname.slice(3) : '';
            const matches = knownPages.filter((row) => row.pageId.startsWith(current));
            const target = route.startsWith('/pages/')
              ? validPagePrefix(current) && matches.length === 1 && matches[0].pageId === id
                ? current
                : shortPageId(
                    id,
                    knownPages.map((row) => row.pageId),
                  )
              : id;
            const tail = hint
              ? hintFragment(hint)
              : target === current && location.hash.startsWith('#t=')
                ? location.hash
                : '';
            return `/p/${target}${tail}`;
          },
        })
      : createHashHistory();
  if (space) keepListSearch(history);
  const router = createRouter({
    routeTree: root.addChildren([home, allPages, page, shortPage, colabSettings, blocked]),
    history,
    ...(inShell() ? { basepath: '/colab' } : {}),
    context: { transport, space, rememberPages: (pages) => (knownPages = pages) },
  });
  router.subscribe('onResolved', () => void buildWatch.check());
  return router;
}
declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAppRouter>;
  }
}
