import {
  useCallback,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ComponentPropsWithoutRef,
  type ReactNode,
} from 'react';
import { BrowserAction } from '@tmt/browser-ui/react';
import { text } from '../strings.js';
import { useNotificationConversation } from '../reply-notification-context.js';

const bottomThreshold = 24;
function atBottom(node: HTMLDivElement) {
  return node.scrollHeight - node.clientHeight - node.scrollTop <= bottomThreshold;
}
function scrollPosition(node: HTMLDivElement) {
  return { top: node.scrollTop, height: node.clientHeight, contentHeight: node.scrollHeight };
}
/** Owns history scrolling; callers retain placement, draft and action lifetimes. */
export function ConversationWindow({
  title,
  collapsed = false,
  summary,
  hideHeader = false,
  historyLabel,
  caption,
  actions,
  messageIds,
  ownWriter,
  historyClassName = '',
  notice,
  composer,
  overlay,
  notificationDestination,
  children,
  className = '',
  ...attributes
}: Omit<ComponentPropsWithoutRef<'section'>, 'title'> & {
  title: ReactNode;
  collapsed?: boolean;
  summary?: ReactNode;
  hideHeader?: boolean;
  historyLabel?: string;
  caption?: string;
  actions: ReactNode;
  messageIds: readonly string[];
  ownWriter?: string;
  historyClassName?: string;
  notice?: ReactNode;
  composer?: ReactNode;
  overlay?: ReactNode;
  notificationDestination?: string;
}) {
  if (hideHeader && !historyLabel)
    throw new Error('A headerless conversation requires a history label');
  const headingId = useId();
  const history = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const arrivals = useRef<HTMLDivElement>(null);
  const previous = useRef<Set<string>>(undefined);
  const following = useRef(true);
  const position = useRef<ReturnType<typeof scrollPosition> | undefined>(undefined);
  const [pending, setPending] = useState(false);
  const notification = useNotificationConversation(notificationDestination);
  const notificationRef = useRef(notification);
  const revealFrame = useRef<number>(undefined);
  notificationRef.current = notification;
  const clearPending = useCallback(() => {
    // Removing a focused action must not drop keyboard focus to the document.
    if (arrivals.current?.contains(document.activeElement))
      history.current?.focus({ preventScroll: true });
    setPending(false);
  }, []);
  const reportReading = useCallback(() => {
    const node = history.current;
    notificationRef.current.report({
      visible:
        !!node &&
        !node.ownerDocument.hidden &&
        node.clientHeight > 0 &&
        node.getClientRects().length > 0,
      atEnd: following.current,
    });
  }, []);
  const revealRecord = useCallback(() => {
    const node = history.current;
    const request = notificationRef.current.reveal;
    if (!node || !request || !node.clientHeight || !node.getClientRects().length) return;
    const record = [...node.querySelectorAll<HTMLElement>('[data-conversation-record]')].find(
      (element) => element.dataset.conversationRecord === request.recordKey,
    );
    if (!record) return;
    node.scrollTop += record.getBoundingClientRect().top - node.getBoundingClientRect().top;
    position.current = scrollPosition(node);
    following.current = atBottom(node);
    if (following.current) clearPending();
    node.focus({ preventScroll: true });
    reportReading();
    notificationRef.current.revealed?.(request.id);
  }, [clearPending, reportReading]);
  const requestReveal = useCallback(() => {
    if (!notificationRef.current.reveal || revealFrame.current !== undefined) return;
    // Native Drawer promotion/focus runs in its parent's layout effect. Reveal
    // after that opening phase rather than racing it from the child effect.
    revealFrame.current = requestAnimationFrame(() => {
      revealFrame.current = undefined;
      revealRecord();
    });
  }, [revealRecord]);
  const latest = useCallback(() => {
    const node = history.current;
    if (!node) return;
    following.current = true;
    node.scrollTop = node.scrollHeight;
    position.current = scrollPosition(node);
    clearPending();
    reportReading();
  }, [clearPending, reportReading]);
  useLayoutEffect(() => {
    const ids = new Set(messageIds);
    const opened = previous.current === undefined;
    const added = [...ids].filter((id) => !previous.current?.has(id));
    const own =
      ownWriter &&
      added.some(
        (id) => id.startsWith(`comment:${ownWriter}:`) || id.startsWith(`ask:${ownWriter}:`),
      );
    previous.current = ids;
    if (opened || own || (added.length && following.current)) latest();
    else if (added.length) setPending(true);
  }, [messageIds, ownWriter, latest]);
  useLayoutEffect(() => {
    requestReveal();
    return () => {
      if (revealFrame.current !== undefined) cancelAnimationFrame(revealFrame.current);
      revealFrame.current = undefined;
    };
  }, [notification.reveal, messageIds, requestReveal]);
  useLayoutEffect(() => {
    const observer = new ResizeObserver(() => {
      // Chat keeps its drawer mounted while closed. A hidden history has no
      // reading position to retain; its next visible size opens at latest.
      const node = history.current;
      if (node && (!node.clientHeight || atBottom(node))) following.current = true;
      if (following.current) latest();
      else if (node) position.current = scrollPosition(node);
      requestReveal();
      reportReading();
    });
    if (history.current) observer.observe(history.current);
    if (content.current) observer.observe(content.current);
    document.addEventListener('visibilitychange', reportReading);
    reportReading();
    return () => {
      observer.disconnect();
      document.removeEventListener('visibilitychange', reportReading);
    };
  }, [latest, requestReveal, reportReading]);
  return (
    <section {...attributes} className={`conversation-window ${className}`}>
      {!hideHeader && (
        <header className="conversation-window-bar thread-bar">
          <div className="conversation-heading">
            <h2 className="conversation-title" id={headingId}>
              {title}
            </h2>
            {caption && <span className="conversation-caption">{caption}</span>}
          </div>
          <span className="thread-bar-actions">{actions}</span>
        </header>
      )}
      {notice}
      {collapsed && summary}
      <div className="conversation-history" hidden={collapsed}>
        <div
          className={`conversation-messages ${historyClassName}`}
          ref={history}
          role="region"
          aria-labelledby={historyLabel ? undefined : headingId}
          aria-label={historyLabel}
          tabIndex={-1}
          onScroll={() => {
            const node = history.current;
            if (!node) return;
            const previousPosition = position.current;
            const currentPosition = scrollPosition(node);
            position.current = currentPosition;
            // A queued scroll may reflect a layout clamp before size observation.
            // Only ignore the offset that the shrinking range can clamp to, using
            // either content/viewport ordering. A reader's different offset still
            // detaches even when a resize is pending; size observation owns following.
            if (
              !previousPosition ||
              currentPosition.top ===
                Math.min(
                  previousPosition.top,
                  Math.max(
                    0,
                    Math.min(previousPosition.contentHeight, currentPosition.contentHeight) -
                      Math.max(previousPosition.height, currentPosition.height),
                  ),
                )
            )
              return;
            following.current = atBottom(node);
            if (following.current) clearPending();
            reportReading();
          }}
        >
          <div className="conversation-message-content" ref={content}>
            {children}
          </div>
        </div>
        <div
          className="conversation-arrivals"
          ref={arrivals}
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >
          {pending && (
            <BrowserAction
              type="button"
              variant="text"
              label={text.newMessages}
              onActivate={(event) => {
                if (event.isTrusted) latest();
              }}
            />
          )}
        </div>
      </div>
      {composer && (
        <div className="conversation-composer" hidden={collapsed}>
          {composer}
        </div>
      )}
      {overlay}
    </section>
  );
}
