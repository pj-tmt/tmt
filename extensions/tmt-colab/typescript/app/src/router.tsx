import { PageMenu as BrowserMenu } from './page-menu.js';
import { useEditLockLayer } from './edit-lock-layer.js';
import { ShellPage } from './shell-page.js';
import { PageWorkBand } from './work-band-live.js';
import { usePageLinks } from './page-link-bar.js';
import { usePageLayers } from './page-layers.js';
import { AgentActivityProvider } from './components/agent-activity.js';
import { RemoteAttention, RemotePresenceProvider } from './remote-header.js';
import { PagePinNotice, PageProjects } from './page-projects.js';
import type { PagePinRequest, PagePinState } from './pins.js';
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
import {
  ArrowUpRight,
  ChevronLeft,
  ChevronRight,
  MessageCircle,
  MessageSquarePlus,
  MessagesSquare,
  MousePointer2,
} from 'lucide-react';
import { moreButtonRect, PageHeaderActions, type PageHeaderAction } from './page-header-actions.js';
import { AppHeader } from './app-header.js';
import { SharedTopBar } from './shared-top-bar.js';
import { ColabHeader } from './colab-header.js';
import { creatorName, PageAttribution } from './page-attribution.js';
import { PageLayoutSection } from './page-layout-section.js';
import { ErrorDetails, NoticeCard } from './notice-card.js';
import { openFailure } from './open-failure.js';
import {
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
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
import {
  embeddedPageEnvironment,
  PageEnvironmentProvider,
  usePageEnvironment,
} from './page-environment.js';
import { ColabSettings } from './colab-settings.js';
import { useQuickSwitcher } from './quick-switcher-context.js';
import { NeedsYouHeader } from './needs-you-header.js';
import { shareView } from './share-view.js';
import { pendingNeedsYou } from './needs-you.js';
import { useReplyNotifications } from './use-reply-notifications.js';
import { ReplyNotificationObserver } from './reply-notifications.js';
import { ReplyNotificationContext } from './reply-notification-context.js';
import { ThreadPanel, ThreadWindow } from './thread-panel.js';
import { ResolveAllThreads } from './resolve-all-threads.js';
import { presentationOf } from './thread-status-presentation.js';
import { isStatusThread, openThreadCount } from './thread-status-view.js';
import {
  firstThread,
  listedThreads,
  threadKey,
  threadNeedsYou,
  type ThreadFilter,
} from './comments-listing.js';
import { useThreadFilter } from './use-thread-filter.js';
import type { QuoteSelector } from './thread-records.js';
import { Home, validateHomeSearch, type HomeSearch } from './home.js';
import { knownScope, readScope } from './project-scope.js';
import { ManagePage } from './manage-page.js';
import './about-drawer.css';
import { mountRenderer, MAX_RENDER_SOURCE_BYTES } from './renderer.js';
import { framedLayout } from './page-layout.js';
import type { AnchorPosition, RenderState, SelectionRect } from './renderer.js';
import { text } from './strings.js';
import type { ConnectionState, EndedAction } from './connection-model.js';
import { useConnection } from './use-connection.js';
import { terminalFailure } from './terminal-failure.js';
import { buildWatch } from './app-build.js';
import { SessionEvictedError } from './ask-remote.js';
import { SessionLimitError } from './session-limit-error.js';
import { SessionLimitBody } from './session-limit.js';
import { ExportPanel } from './export-panel.js';
import { BrowserDrawer, BrowserPanel } from '@tmt/browser-ui/react';
import { AgentStatusPanel } from './agent-status-panel.js';
import { AgentPresentation, useAgentPresentation } from './agent-directory.js';
import { ChatPanel } from './chat-panel.js';
import { dockPanel, useDockMotion } from './dock-motion.js';
import { AttachmentDropPanel } from './attachment-drop.js';
import { FilesSurface } from './files-panel.js';
import { MetadataDialogs } from './metadata-dialogs.js';
import { verifiedQuestionStates } from './question-registry.js';
import { CreatorDefaultProvider, creatorTarget } from './creator-default.js';
import { CHAT_DRAFT, SavedDrafts, savedDrafts } from './saved-drafts.js';
import { isChatThread, threadNumbers } from './thread-records.js';
import { PageSizeSection, usePageSize } from './page-size.js';
import { SourceDraft } from './use-page-editor.js';
import { PageSource } from './page-source.js';
import { usePagePanels } from './use-page-panels.js';
import { usePanelMode } from './panel-mode.js';
import './panel-layout.css';
import { useAnnotations } from './use-annotations.js';
import { usePageRenderer } from './use-page-renderer.js';

/** Controls in the page and above it that the floating actions move above, never over: the cards'
 * footers, and the open work band's controls, which can fill a phone's viewport. */
const AVOIDED_CARDS = '.question-group-footer, .proposal-inline .proposal-card';
const AVOIDED_BAND =
  '.work-band :is(button, a[href], input, select, textarea, [role="button"], [role="combobox"], [role="menuitem"])';

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
  comments,
  unseen,
  commentsButton,
  toggleComments,
  onPlacement,
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
  /** The open threads; absent while the page's threads are still being verified: a place held, not a false 0. */
  comments?: number;
  /** Some thread has news for the viewer. */
  unseen: boolean;
  commentsButton: Ref<HTMLButtonElement>;
  toggleComments(): void;
  onPlacement(value: { right: number; bottom: number; height: number }): void;
  canSelect: boolean;
  /** The button's place while the threads are unverified: fenced, not greyed. */
  selectPending: boolean;
}) {
  const element = useRef<HTMLDivElement>(null);
  const dock = useRef<HTMLDivElement>(null);
  const tooltipPlacement = useMemo(
    () => ({ anchor: () => dock.current, side: 'above' as const }),
    [],
  );
  const [placement, setPlacement] = useState<{
    right: number;
    bottom: number;
    visibility: 'visible' | 'hidden';
  }>({ right: 24, bottom: 24, visibility: 'hidden' });
  useEffect(() => {
    const workspace = host?.closest('.workspace');
    const place = () => {
      const box = host?.getBoundingClientRect();
      if (!box) return;
      const margin = innerWidth <= 640 ? 16 : 24;
      // These parent-owned overlays sit outside the renderer host's layout box.
      // Their actual bounds keep the control clear without altering author geometry.
      // A page hosted in a panel sits inside that panel's drawer; only another drawer is an overlay.
      const drawer = [
        ...document.querySelectorAll<HTMLDialogElement>(
          '.tmt-ui-drawer[open]:not([data-panel="chat"])',
        ),
      ]
        .find((candidate) => !candidate.contains(host))
        ?.getBoundingClientRect();
      // A card sits over the page's right edge and a sheet over its bottom; a column is a grid
      // track beside the page, so the host's own box already ends before it.
      const floating = document.querySelector<HTMLElement>(
        '.tmt-ui-panel:not([hidden]):not([data-floating-discussion])[data-mode="card"], .tmt-ui-panel:not([hidden]):not([data-floating-discussion])[data-mode="sheet"]',
      );
      const over = floating?.getBoundingClientRect();
      const right = Math.min(
        box.right,
        drawer?.left ?? innerWidth,
        over && floating?.dataset.mode === 'card' ? over.left : innerWidth,
      );
      let bottom = Math.min(
        innerHeight,
        box.bottom,
        over && floating?.dataset.mode === 'sheet' ? over.top : innerHeight,
      );
      const fab = element.current?.getBoundingClientRect();
      const left = right - margin - (fab?.width ?? 0);
      const avoided = [
        ...(workspace?.querySelectorAll<HTMLElement>(`${AVOIDED_CARDS}, ${AVOIDED_BAND}`) ?? []),
      ]
        .filter((footer) => !footer.closest('[data-detached],[hidden]'))
        .map((footer) =>
          (footer.closest('.question-compact-block') ?? footer).getBoundingClientRect(),
        )
        .filter(
          (footer) =>
            footer.width > 0 && footer.height > 0 && footer.bottom > 0 && footer.top < innerHeight,
        )
        .sort((a, b) => b.top - a.top);
      // Moving above one control can land on the next one up: settle in a few passes. With no room
      // left above them the group hides below, as for the cards.
      for (let pass = 0; pass < 4; pass++) {
        let moved = false;
        for (const footer of avoided) {
          if (
            left < footer.right &&
            right - margin > footer.left &&
            bottom - margin > footer.top &&
            bottom - margin - (fab?.height ?? 0) < footer.bottom
          ) {
            bottom = footer.top + margin - 8;
            moved = true;
          }
        }
        if (!moved) break;
      }
      // The work band is pinned right under the page bar: above its top there is only chrome.
      const ceiling = Math.max(
        0,
        workspace?.querySelector('.work-band')?.getBoundingClientRect().top ?? 0,
      );
      const next = {
        right: Math.max(margin, innerWidth - right + margin),
        bottom: innerHeight - bottom + margin,
        visibility:
          bottom - margin - (fab?.height ?? 0) >= ceiling + 8
            ? ('visible' as const)
            : ('hidden' as const),
      };
      setPlacement(next);
      onPlacement({ right: next.right, bottom: next.bottom, height: fab?.height ?? 0 });
    };
    place();
    const observer = new ResizeObserver(place);
    if (host) observer.observe(host);
    if (element.current) observer.observe(element.current);
    // The band opens and closes above the host without resizing it: the workspace's own size follows.
    if (workspace) observer.observe(workspace);
    for (const card of workspace?.querySelectorAll(`${AVOIDED_CARDS}, ${AVOIDED_BAND}`) ?? [])
      observer.observe(card);
    // Native dialog show/close happens in its own effect. Follow its actual layout,
    // rather than depending on sibling effect order.
    for (const drawer of document.querySelectorAll('.tmt-ui-drawer, .tmt-ui-panel'))
      observer.observe(drawer);
    const composer = document.querySelector('.annotation-popover');
    if (composer) observer.observe(composer);
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, { passive: true });
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place);
    };
  }, [host, enabled, panel, discussionOpen, slots, onPlacement]);
  return (
    <div ref={element} className="selection-mode" style={placement}>
      {enabled && !discussionOpen && (
        <p className="selection-mode-hint">{text.selectionModeHint}</p>
      )}
      <div className="page-floating-actions" ref={dock}>
        {(canSelect || selectPending) && (
          <BrowserIconAction
            type="button"
            variant="secondary"
            tooltipPlacement={tooltipPlacement}
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
        {!enabled && !discussionOpen && (
          <BrowserIconAction
            type="button"
            variant="secondary"
            tooltipPlacement={tooltipPlacement}
            label={chatUnread ? `${text.chat} · ${text.chatUnread}` : text.chat}
            buttonRef={chatButton}
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
        {!enabled && !discussionOpen && (
          <BrowserIconAction
            type="button"
            variant="secondary"
            tooltipPlacement={tooltipPlacement}
            label={`${text.comments}${comments === undefined ? '' : ` (${comments})`}${unseen ? ` · ${text.threadUnseen}` : ''}`}
            buttonRef={commentsButton}
            expanded={panel === 'comments'}
            icon={
              <>
                <MessagesSquare />
                {comments !== undefined ? (
                  <span className="dock-count" data-unseen={unseen || undefined}>
                    {comments}
                    {unseen && <span aria-hidden="true">•</span>}
                  </span>
                ) : (
                  <span className="dock-count" data-pending aria-hidden="true" />
                )}
              </>
            }
            onActivate={(event) => {
              if (event.isTrusted) toggleComments();
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
  } | null>(null);
  useEffect(() => {
    const notice = noticeVisible
      ? host?.parentElement?.querySelector<HTMLElement>('.tmt-ui-notice')
      : null;
    const place = () => {
      const frame = host?.querySelector('iframe')?.getBoundingClientRect();
      const width = expanded ? Math.min(440, innerWidth - 24) : 34;
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
      const placementHeight = expanded ? Math.min(480, edge - inset - 4) : 34;
      if (expanded && innerWidth <= 640) {
        const noticeBox = notice?.getBoundingClientRect();
        const below = Math.max(0, edge - Math.max(inset, noticeBox?.bottom ?? inset) - 8);
        const above = Math.max(0, (noticeBox?.top ?? inset) - inset - 16);
        // A failure card can fill the room below the page chrome. Preserve a usable draft
        // above it instead of collapsing the retained window to zero height beneath it.
        setPosition(
          above > below
            ? {
                left: 12,
                top: inset + 8,
                maxHeight: above,
              }
            : {
                left: 12,
                bottom: Math.max(8, innerHeight - edge),
                maxHeight: below,
              },
        );
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
      if (!expanded && innerWidth > 640) {
        // The Annotate control hangs from the selection's lower right corner, with no line to the
        // text; with no room under the selection it sits just above it. On a phone the system's own
        // selection handles and menu occupy that corner, so the control keeps its place beside or
        // under the selection below.
        const below = selectionBottom + 2 + placementHeight <= edge;
        setPosition({
          left: Math.max(
            8,
            frame.left,
            Math.min(innerWidth - width - 8, frame.right - width - 8, right - 6),
          ),
          ...(below
            ? { top: selectionBottom + 2, maxHeight: placementHeight }
            : { bottom: innerHeight - selectionTop + 2, maxHeight: placementHeight }),
        });
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
      data-testid={children ? undefined : 'selection-ask'}
      // The control never takes the pointer from the selection it annotates.
      onPointerDown={children ? undefined : (event) => event.preventDefault()}
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
          visibility: position ? 'visible' : 'hidden',
        } as React.CSSProperties
      }
    >
      {children ?? (
        <BrowserIconAction
          type="button"
          variant="primary"
          label={text.annotationTitle}
          icon={<MessageSquarePlus aria-hidden />}
          ariaKeyShortcuts="c Alt+Enter"
          onActivate={(event) => {
            if (event.isTrusted) open();
          }}
        />
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
    // The heading takes focus once it is drawn: in the shared bar it mounts after this view.
    const header = useCallback((node: HTMLElement | null) => {
      const title = node?.matches('h1') ? node : node?.querySelector('h1');
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
          <SharedTopBar value={hostSlot.topBarReady()}>{view}</SharedTopBar>
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
  rememberPages(context.transport, context.space, home.pages);
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
        page
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

function Shell() {
  const { transport, space } = root.useRouteContext();
  const router = useRouter();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const isPage = pathname.startsWith('/pages/');
  const isHome = pathname === '/';
  // Colab's own shell has the shared bar's Search on every route, and the bar owns the one shortcut.
  const hostSlot = useContext(ShellHostSlot);
  const ownShell = hostSlot === null;
  const sharedSearch = ownShell || hostSlot.topBarReady();
  return (
    <RemotePresenceProvider owner={transport.remotePresence}>
      <QuickSwitcherHost
        transport={transport}
        space={space}
        pageId={isPage ? pathname.slice('/pages/'.length) : undefined}
        routeKey={pathname}
        allowed={ownShell || isPage || isHome}
        shortcut={!sharedSearch}
        navigate={(pageId) => router.navigate({ to: '/pages/$pageId', params: { pageId } })}
        pageHref={(pageId) =>
          router.history.createHref(
            router.buildLocation({ to: '/pages/$pageId', params: { pageId } }).publicHref,
          )
        }
      >
        <ColabShell transport={transport} space={space}>
          {/* The list names its own scope; a page draws its own header. */}
          {!isPage && !isHome && (
            <AppHeader
              title={
                pathname.startsWith('/short/')
                  ? text.openAPage
                  : pathname === '/settings'
                    ? text.colabSettings.section
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

/** A chapter's page hosted in the shell's panel: it opens the page through the transport as the
 * route would, and gives its live connection up when the panel leaves it (back to the summary, the
 * full page, Esc), so one page never holds two. */
function EmbeddedPage({ pageId }: { pageId: string }) {
  const router = useRouter();
  const { transport, space } = root.useRouteContext();
  const host = usePageEnvironment();
  const [loaded, setLoaded] = useState<PageSnapshot | 'failed'>();
  useEffect(() => {
    const abort = new AbortController();
    let opened: PageSnapshot | undefined;
    setLoaded(undefined);
    transport.page(pageId, abort.signal).then(
      (snapshot) => {
        if (abort.signal.aborted) snapshot.binding?.close();
        else {
          opened = snapshot;
          setLoaded(snapshot);
        }
      },
      () => {
        if (!abort.signal.aborted) setLoaded('failed');
      },
    );
    return () => {
      abort.abort();
      opened?.binding?.close();
    };
  }, [transport, pageId]);
  const routing = useMemo<PageRouting>(
    () => ({ ...routePageRouting(router), home: (brand) => brand, scopeCrumb: () => null }),
    [router],
  );
  const wrap = useRef<HTMLDivElement>(null);
  const environment = useMemo(() => embeddedPageEnvironment(host, () => wrap.current), [host]);
  return (
    <div className="shell-embedded" ref={wrap}>
      {loaded === undefined ? (
        <p className="shell-state" role="status">
          <BusyMark /> {text.loading}
        </p>
      ) : loaded === 'failed' ? (
        <p className="shell-state" role="status">
          {text.blocked}
        </p>
      ) : (
        <PageEnvironmentProvider value={environment}>
          <OpenPage
            key={pageId}
            embedded
            transport={transport}
            space={space}
            loaded={loaded}
            search={{}}
            scope={undefined}
            routing={routing}
          />
        </PageEnvironmentProvider>
      )}
    </div>
  );
}

function OpenPage({
  transport,
  space,
  loaded,
  search,
  scope,
  routing,
  embedded = false,
}: {
  transport: PageTransport;
  space: string | undefined;
  loaded: PageSnapshot;
  /** The reading, element or task a link or a shell item opened the page at. */
  search: { read?: string; anchor?: string; task?: string };
  scope: string | undefined;
  routing: PageRouting;
  /** The page is hosted in a panel of another page: its drawers and menus stay the app's, but the
   * panel already names it, so it keeps only its actions and status. */
  embedded?: boolean;
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
  const [workBandSize, setWorkBandSize] = useState(0);
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
  const panelMode = usePanelMode();
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
  const chatOpener = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (panel !== 'chat') chatOpener.current = null;
  }, [panel]);
  const floatingComments = useRef<HTMLButtonElement>(null);
  const discussionMarkerOpener = useRef<string | null>(null);
  useEffect(() => {
    if (panel !== 'comments') discussionMarkerOpener.current = null;
  }, [panel]);
  const [dockPlacement, setDockPlacement] = useState({ right: 24, bottom: 24, height: 44 });
  const updateDockPlacement = useCallback(
    (next: { right: number; bottom: number; height: number }) => {
      setDockPlacement((previous) =>
        previous.right === next.right &&
        previous.bottom === next.bottom &&
        previous.height === next.height
          ? previous
          : next,
      );
    },
    [],
  );
  const closeChat = () => {
    const opener = chatOpener.current;
    (opener?.isConnected ? opener : floatingChat.current)?.focus({ preventScroll: true });
    chatOpener.current = null;
    close();
  };
  const closeComments = () => {
    const marker = discussionMarkerOpener.current;
    const currentRenderer = renderer.current;
    discussionMarkerOpener.current = null;
    floatingComments.current?.focus({ preventScroll: true });
    close();
    if (marker)
      requestAnimationFrame(() => {
        if (panelsRef.current.panel === null) currentRenderer?.focusThread(marker);
      });
  };
  useDockMotion(dockPanel(panel));
  const activateChat = useRef<() => boolean>(() => false);

  // The page's own projects: the picker opens from its chips and from Projects… in the More menu.
  const [projects, setProjects] = useState<{ request: number; from?: DOMRect }>({ request: 0 });
  const openProjects = (from?: DOMRect) =>
    setProjects((previous) => ({ request: previous.request + 1, from }));
  const [metadataDialog, setMetadataDialog] = useState<'creator' | 'questions' | null>(null);
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
    creator: snapshot.creator,
    questions: snapshot.questions,
    creationRecipient: snapshot.creationRecipient,
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
  // The Source draft lives outside the route's state: only the Source drawer renders as it changes.
  const [sourceDraft] = useState(() => new SourceDraft(snapshot));
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
    JSON.stringify({
      asks: (view.asks ?? []).map(({ writer, operationId }) => [writer, operationId]).sort(),
      locks: (view.editLocks ?? [])
        .map((lock) => [lock.holder.id, lock.lockId, lock.revision])
        .sort(),
    }),
  );
  const archived = view.archived ?? snapshot.archived ?? false;
  const wasArchived = useRef(archived);
  const [archiveAnnouncement, setArchiveAnnouncement] = useState('');
  useEffect(() => {
    if (archived && !wasArchived.current) setArchiveAnnouncement(text.archiveAnnouncement);
    wasArchived.current = archived;
  }, [archived]);
  useEffect(() => {
    sourceDraft.adoptSnapshot(snapshot);
    setView({
      discussionPending: snapshot.discussionPending,
      pageType: snapshot.pageType,
      shell: snapshot.shell,
      work: snapshot.work,
      archived: snapshot.archived,
      source: snapshot.source,
      title: snapshot.title,
      originalAuthor: snapshot.originalAuthor,
      creator: snapshot.creator,
      questions: snapshot.questions,
      creationRecipient: snapshot.creationRecipient,
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
        sourceDraft.adoptUpdate(value.source);
      },
      (error) => connectionOwner.report(error),
    );
    return () => {
      unsubscribe?.();
    };
  }, [snapshot, loaded, transport, connectionOwner, replyObserver, sourceDraft]);
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
  } = useAnnotations({
    snapshot,
    transport,
    binding,
    renderer,
    host,
    panels: {
      clear,
      showThread,
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
  // The Comments drawer shows one thread's Discussion view while a thread of this page is selected.
  const discussedThread =
    panel === 'comments' &&
    activeThread !== null &&
    (view.threads ?? []).some(
      (thread) =>
        !isChatThread(thread) && `${thread.ref.writer}:${thread.threadId}` === activeThread,
    );
  // The Discussion header's arrows walk the threads in the order of their numbers, the order of the
  // position it shows.
  const [resolveSlot, setResolveSlot] = useState<HTMLElement | null>(null);
  const [resolveAllSlot, setResolveAllSlot] = useState<HTMLElement | null>(null);
  const [batchStatus, setBatchStatus] = useState<{ generation: number; content: ReactNode } | null>(
    null,
  );
  const [batchGeneration, setBatchGeneration] = useState(0);
  const [threadFilter, setThreadFilter] = useThreadFilter();
  useEffect(() => {
    setBatchStatus(null);
    setBatchGeneration((generation) => generation + 1);
  }, [panel, activeThread, threadFilter]);
  const listed = useMemo(
    () =>
      listedThreads(
        view.threads ?? [],
        numbers,
        threadFilter,
        activeThread,
        view.threadPresentations,
      ),
    [view.threads, numbers, threadFilter, activeThread, view.threadPresentations],
  );
  useEffect(() => {
    if (panel !== 'comments' || discussionPending || changingStatus) return;
    if (activeThread && listed.some((thread) => threadKey(thread) === activeThread)) return;
    const first = firstThread(listed, view.threadPresentations);
    // Passive projection changes select cosmetically; only a trusted open acknowledges status.
    if (first) setActiveThread(threadKey(first));
    else if (activeThread) setActiveThread(null);
  }, [
    panel,
    discussionPending,
    changingStatus,
    activeThread,
    listed,
    view.threadPresentations,
    setActiveThread,
  ]);
  const position =
    activeThread === null ? -1 : listed.findIndex((t) => threadKey(t) === activeThread);
  const neighbour = (offset: -1 | 1) => (position < 0 ? undefined : listed[position + offset]);
  const openNeighbour = (offset: -1 | 1) => {
    const thread = neighbour(offset);
    if (thread) openThread(thread.ref);
  };
  const openThreads = discussionPending ? undefined : openThreadCount(view.threads ?? []);
  const fileCount = view.attachments?.length ?? 0;
  const unseenThreads = (view.threadPresentations ?? []).some((value) => value.status.unseen);
  const statusCoordinator =
    archived || liveError?.message === managementChanged ? undefined : snapshot.binding?.status;
  const [state, setState] = useState<RenderState | 'loading'>('loading');
  // Counts sources the open frame applied in place (#5022): what the page draws over the frame is
  // placed again for each, as it is when a fresh frame becomes ready.
  const [contentRevision, setContentRevision] = useState(0);
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
    if (insertMention)
      chatOpener.current =
        document.activeElement instanceof HTMLElement && document.activeElement !== document.body
          ? document.activeElement
          : null;
    openChat(
      !discussionBlocked && !recoveryRequired && !chatBusy.current
        ? { id: crypto.randomUUID(), insertMention }
        : undefined,
    );
    return true;
  };
  escapeAnnotation.current = () => {
    if (panel === 'chat') {
      if (!chatBusy.current) closeChat();
    } else if (panel === 'comments') {
      if (!annotationBusy.current && !statusBusy.current) closeComments();
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
        (event.code === 'KeyC' && !canSelect)
      )
        return;
      if (event.key === '@') {
        if (activateChat.current()) event.preventDefault();
        return;
      }
      if (
        event.key === 'Escape' &&
        !panelsRef.current.panel &&
        !annotationRef.current &&
        !selectionMode
      )
        return;
      event.preventDefault();
      if (event.key === 'Escape') escapeAnnotation.current();
      else if (!selectionMode && currentSelector.current && currentRectangle.current) annotate();
      else toggleSelectionMode.current();
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [canSelect, selectionMode, annotate, annotationRef, currentSelector, currentRectangle]);
  const { read: readingHint, anchor } = search;
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
    contentRevision,
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
    setActiveThread,
  });
  const editLockLayer = useEditLockLayer({
    page: snapshot.id,
    view,
    agents,
    renderer,
    host,
    toolbar,
    state,
    contentRevision,
    admitted: !discussionPending && !liveError && !archived,
  });
  const { resolved, anchorsChecked, pageScheme, missingAnchor } = usePageRenderer({
    snapshot,
    view,
    endedError,
    host,
    renderer,
    setState,
    toolbar,
    workBandSize,
    anchor,
    layers: {
      onEditLocks: editLockLayer.onEditLocks,
      onSlots: inlineSlots.onSlots,
      onQuestions: onQuestionAnchors,
      onReadings: onReadingAnchors,
    },
    resetPageLinks,
    requestPageLink,
    onContentApplied: () => setContentRevision((count) => count + 1),
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
        if (thread) {
          if (panelsRef.current.panel !== 'comments') discussionMarkerOpener.current = id;
          openThread(thread.ref);
        }
      },
      onOpenList: (id) => {
        if (panelsRef.current.panel !== 'comments') discussionMarkerOpener.current = id;
        if (annotationRef.current) closeRef.current(false);
        panelsRef.current.show('comments');
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
  // A resolved thread leaves the marker set, but while its Discussion view is open the page keeps a
  // quiet mark on its passage, so the person can see and scroll to what it was about.
  const quietCandidate =
    discussedThread && activeThread !== null
      ? (view.threads ?? []).find(
          (thread) =>
            !isChatThread(thread) &&
            `${thread.ref.writer}:${thread.threadId}` === activeThread &&
            thread.resolved &&
            !thread.deleted &&
            thread.anchor,
        )
      : undefined;
  const quietRef = useRef<{ id: string; selector: QuoteSelector } | undefined>(undefined);
  quietRef.current = shareView(
    quietRef.current,
    quietCandidate?.anchor ? { id: activeThread!, selector: quietCandidate.anchor } : undefined,
  );
  const quietThread = quietRef.current;
  useEffect(() => {
    if (state !== 'ready') return;
    renderer.current?.highlight(
      anchoredThreads as { id: string; selector: QuoteSelector; ordinal: number }[],
      annotation?.selector ?? undefined,
      threadTotal,
      quietThread,
    );
  }, [state, contentRevision, anchoredThreads, annotation, threadTotal, quietThread]);
  // The passage of the thread whose window or Discussion view is open takes the page's selected look; a window for a new
  // quote or none leaves every passage at rest. The anchors are a dependency so that a thread opened
  // as it is created is selected once the frame holds its passage.
  const openThreadId = annotation?.thread
    ? `${annotation.thread.writer}:${annotation.thread.id}`
    : discussedThread
      ? activeThread
      : null;
  useEffect(() => {
    if (state !== 'ready') return;
    renderer.current?.select(openThreadId);
  }, [state, contentRevision, anchoredThreads, openThreadId]);
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
      } else if (event.thread) openThread(event.thread);
      else return false;
      return true;
    },
  });
  const filterLabels: Record<ThreadFilter, string> = {
    open: text.commentsViewOpen,
    needs: text.needsYouWord,
    all: text.commentsViewAll,
  };
  const threadFilterMenu = (
    <BrowserMenu
      label={text.commentsFilter}
      triggerText={filterLabels[threadFilter]}
      disabled={changingStatus}
      items={(['open', 'needs', 'all'] as const).map((key) => ({
        key,
        label: filterLabels[key],
        role: 'menuitemradio' as const,
        checked: key === threadFilter,
      }))}
      onSelect={(filter) => {
        setThreadFilter(filter);
        const first = firstThread(
          listedThreads(view.threads ?? [], numbers, filter, null, view.threadPresentations),
          view.threadPresentations,
        );
        if (first) openThread(first.ref, { revealInPage: false });
        else setActiveThread(null);
      }}
    />
  );
  const content = (
    <section
      className="page"
      style={{ '--colab-work-band-size': `${workBandSize}px` } as React.CSSProperties}
    >
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
        embedded={embedded}
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
        barRef={toolbar}
        page
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
                  items={pendingNeedsYou(
                    [
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
                      ...(view.questions === undefined
                        ? questionLayer.navigation
                        : (verifiedQuestionStates(view) ?? [])
                            .filter((question) => question.state === 'open')
                            .map((question) => ({
                              kind: 'answer' as const,
                              state: 'unanswered' as const,
                              id: question.id,
                              title: question.title ?? question.id,
                              agents: [
                                question.agent?.name ?? view.creator?.agent.name ?? '',
                              ].filter(Boolean),
                            }))),
                      ...reviewLayer.navigation,
                    ],
                    view.questions !== undefined,
                  )}
                  onPick={(item) => {
                    if (item.kind === 'proposal') {
                      if (!proposalLayer.jump(item.id)) {
                        const ref = proposalLayer.reference(item.id);
                        if (ref) openThread(ref);
                      }
                    } else if (item.kind === 'reading') readingLayer.jump(item.id, true);
                    else if (item.kind === 'answer') {
                      if (!questionLayer.open(item.targetId ?? item.id))
                        setMetadataDialog('questions');
                    } else if (item.kind === 'review') reviewLayer.jump(item.id);
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
            files={fileCount}
            readOnly={archived}
            access={`${archived ? '○ ' : `${text[snapshot.sharing]} · `}${accessLabel}`}
            canExport={!!snapshot.binding && !liveError && !archived}
            canManage={!!transport.management}
            canFiles={!!snapshot.binding?.files}
            canProjects={!!transport.management}
            pin={pin}
            creatorLabel={creatorName(view)}
            onSelect={(action) => {
              if (action === 'creator' || action === 'questions') {
                close();
                setMetadataDialog(action);
              } else if (action === 'projects') openProjects(moreButtonRect());
              else if (action === 'pin') requestPin('toggle');
              else toggle(action);
            }}
          />
        }
      />
      <MetadataDialogs
        key={snapshot.id}
        kind={metadataDialog}
        view={view}
        binding={snapshot.binding}
        onDismiss={() => setMetadataDialog(null)}
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
        <section aria-labelledby="about-author-heading">
          <h3 id="about-author-heading">{text.aboutAuthor}</h3>
          <PageAttribution
            creator={view.creator}
            creationRecipient={view.creationRecipient}
            originalAuthor={view.originalAuthor}
            publisherAgent={view.publisherAgent}
          />
        </section>
        <PageLayoutSection layout={view.layout} reason={view.layoutReason} />
        <PageSizeSection size={size} exportPage={() => show('export')} />
        <section aria-labelledby="about-scripts-heading">
          <h3 id="about-scripts-heading">{text.aboutScripts}</h3>
          <p>{text.warning}</p>
        </section>
      </BrowserDrawer>
      <div className="workspace">
        {pageLinkBar}
        <PageWorkBand
          onSize={setWorkBandSize}
          work={view.work}
          pageState={view.state === undefined ? (view.pageState ?? 'todo') : view.state}
          binding={snapshot.binding}
          transport={transport}
          space={space}
          pageId={snapshot.id}
          title={view.title}
          pageHref={routing.pageHref}
        />
        <div className="canvas" data-layout={framedLayout(view.layout) ? view.layout : undefined}>
          {missingAnchor && (
            <p className="page-anchor-missing" role="status">
              {text.anchorMissing}
            </p>
          )}
          <div className="frame-host" ref={host} />
          {editLockLayer.layer}
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
        <BrowserPanel
          keepMounted
          focusFallback={environment.focusFallback}
          open={panel === 'comments'}
          mode={panelMode}
          data-floating-discussion=""
          style={
            {
              '--discussion-right': `${dockPlacement.right}px`,
              '--discussion-bottom': `${dockPlacement.bottom + dockPlacement.height + 16}px`,
            } as React.CSSProperties
          }
          closeLabel={text.discussionClose}
          closeTooltipLabel={text.discussionClose}
          title={text.discussion}
          caption={
            <>
              {discussedThread && position >= 0 && (
                <span className="discussion-position">
                  {text.discussionPosition(position + 1, listed.length)}
                </span>
              )}
              {threadFilterMenu}
            </>
          }
          headerActions={
            discussedThread && numbers.get(activeThread!) !== undefined ? (
              <>
                <BrowserIconAction
                  type="button"
                  variant="text"
                  label={text.discussionPrevious}
                  icon={<ChevronLeft />}
                  disabled={!neighbour(-1)}
                  onActivate={(event) => {
                    if (event.isTrusted) openNeighbour(-1);
                  }}
                />
                <BrowserIconAction
                  type="button"
                  variant="text"
                  label={text.discussionNext}
                  icon={<ChevronRight />}
                  disabled={!neighbour(1)}
                  onActivate={(event) => {
                    if (event.isTrusted) openNeighbour(1);
                  }}
                />
                <span className="discussion-separator" aria-hidden="true" />
                <span className="discussion-header-slot" ref={setResolveSlot} />
                <span className="discussion-header-slot" ref={setResolveAllSlot} />
                <span className="discussion-separator" aria-hidden="true" />
              </>
            ) : undefined
          }
          data-panel="comments"
          data-discussion={discussedThread || undefined}
          onDismiss={(reason) => {
            if (reason === 'close') closeComments();
            else escapeAnnotation.current();
          }}
        >
          {endedNotice && <div className="panel-notice">{endedNotice}</div>}
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
            <>
              <ThreadPanel
                connectionPresentation={connectionPresentation}
                archived={archived}
                proposalStatusPending={proposalLayer.proposalStatusPending}
                creationRecipient={creatorTarget(view)}
                hideHeader
                numbers={numbers}
                renderProposal={proposalLayer.renderProposal}
                key={`discussion:${snapshot.id}:${draftsReady}`}
                threads={(view.threads ?? []).filter((thread) => !isChatThread(thread))}
                resolved={resolved}
                anchorsChecked={anchorsChecked}
                selection={selector}
                binding={
                  liveError?.message === managementChanged
                    ? undefined
                    : snapshot.binding?.discussion
                }
                ask={liveError?.message === managementChanged ? undefined : snapshot.binding?.ask}
                title={view.title || snapshot.title}
                asks={view.asks ?? []}
                active={activeThread}
                close={closeComments}
                headerSlot={resolveSlot}
                batchStatus={
                  batchStatus?.generation === batchGeneration ? batchStatus.content : null
                }
                showInPage={(ref) => {
                  // The panel stays: it is beside or over the page, not a modal that hides it.
                  renderer.current?.scrollAnchor(`${ref.writer}:${ref.id}`);
                }}
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
              <ResolveAllThreads
                headerSlot={resolveAllSlot}
                threads={listed.filter(
                  (thread) =>
                    !thread.resolved &&
                    (threadFilter !== 'needs' || threadNeedsYou(thread, view.threadPresentations)),
                )}
                coordinator={
                  (view.threadPresentations ?? []).some((value) => value.status.controllable)
                    ? statusCoordinator
                    : undefined
                }
                blocked={discussionBlocked || changingStatus}
                onReport={(content) =>
                  setBatchStatus(content === null ? null : { generation: batchGeneration, content })
                }
              />
            </>
          )}
        </BrowserPanel>
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
          toggleChat={() => {
            if (panel !== 'chat') chatOpener.current = floatingChat.current;
            toggle('chat');
          }}
          comments={openThreads}
          unseen={unseenThreads}
          commentsButton={floatingComments}
          toggleComments={() => {
            discussionMarkerOpener.current = null;
            const selected = (view.threads ?? []).find(
              (thread) =>
                !thread.deleted && !isChatThread(thread) && threadKey(thread) === activeThread,
            );
            const target = selected ?? firstThread(listed, view.threadPresentations);
            if (panel !== 'comments' && target) openThread(target.ref, { revealInPage: false });
            else toggle('comments');
          }}
          onPlacement={updateDockPlacement}
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
                          annotationThread?.proposal?.proposer ?? creatorTarget(view)
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
        <FilesSurface
          notice={endedNotice}
          focusFallback={environment.focusFallback}
          open={panel === 'files'}
          onDismiss={() => close()}
          descriptors={view.attachments ?? []}
          files={snapshot.binding.files}
          readOnly={archived}
          disabled={!!liveError}
          afterSave={() => sourceDraft.saveSettled ?? Promise.resolve(true)}
        />
      )}
      <PageSource
        draft={sourceDraft}
        binding={snapshot.binding}
        latest={latest}
        size={size}
        open={panel === 'source'}
        notice={endedNotice}
        focusFallback={environment.focusFallback}
        archived={archived}
        disabled={!!liveError}
        onDismiss={() => close()}
        exportPage={() => show('export')}
      />
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
        <BrowserPanel
          keepMounted
          focusFallback={() => floatingChat.current ?? environment.focusFallback()}
          open={panel === 'chat'}
          mode={panelMode}
          data-floating-discussion=""
          data-panel="chat"
          style={
            {
              '--discussion-right': `${dockPlacement.right}px`,
              '--discussion-bottom': `${dockPlacement.bottom + dockPlacement.height + 16}px`,
            } as React.CSSProperties
          }
          title="Chat"
          caption={text.chatVisible}
          onDismiss={(reason) => {
            if (reason === 'close') closeChat();
            else escapeAnnotation.current();
          }}
        >
          {archived && endedNotice && <div className="panel-notice">{endedNotice}</div>}
          {chatOpened && discussionPending && !liveError && (
            <p role="status" className="discussion-pending">
              {text.discussionPending}
            </p>
          )}
          {chatOpened && !discussionPending && (
            <ChatPanel
              connectionPresentation={connectionPresentation}
              archived={archived}
              creationRecipient={creatorTarget(view)}
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
              close={closeChat}
              escape={() => escapeAnnotation.current()}
            />
          )}
        </BrowserPanel>
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
          headerBottom: () => toolbar.current?.getBoundingClientRect().bottom ?? 0,
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
                  page
                  title={view.title || snapshot.title}
                  remote={{ deviceName: transport.backendName }}
                />
                <main className="canvas">
                  {(view.shell || !endedError) && (
                    <ShellPage
                      shell={view.shell}
                      embed={embedded ? undefined : (pageId) => <EmbeddedPage pageId={pageId} />}
                    />
                  )}
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
