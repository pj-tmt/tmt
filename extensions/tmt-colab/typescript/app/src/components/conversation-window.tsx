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

const bottomThreshold = 24;
/** Display identities distinguish comments/user turns from admitted agent replies. */
export function conversationRecordKey(
  kind: 'comment' | 'ask' | 'reply',
  writer: string,
  id: string,
) {
  return `${kind}:${writer}:${id}`;
}

/** Owns history scrolling; callers retain placement, draft and action lifetimes. */
export function ConversationWindow({
  title,
  caption,
  actions,
  messageIds,
  ownWriter,
  historyClassName = '',
  notice,
  composer,
  children,
  className = '',
  ...attributes
}: Omit<ComponentPropsWithoutRef<'section'>, 'title'> & {
  title: ReactNode;
  caption?: string;
  actions: ReactNode;
  messageIds: readonly string[];
  ownWriter?: string;
  historyClassName?: string;
  notice?: ReactNode;
  composer?: ReactNode;
}) {
  const headingId = useId();
  const history = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const arrivals = useRef<HTMLDivElement>(null);
  const previous = useRef<Set<string>>(undefined);
  const following = useRef(true);
  const position = useRef(0);
  const [pending, setPending] = useState(false);
  const clearPending = useCallback(() => {
    // Removing a focused action must not drop keyboard focus to the document.
    if (arrivals.current?.contains(document.activeElement))
      history.current?.focus({ preventScroll: true });
    setPending(false);
  }, []);
  const latest = useCallback(() => {
    const node = history.current;
    if (!node) return;
    following.current = true;
    node.scrollTop = node.scrollHeight;
    position.current = node.scrollTop;
    clearPending();
  }, [clearPending]);
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
    const observer = new ResizeObserver(() => {
      // Chat keeps its drawer mounted while closed. A hidden history has no
      // reading position to retain; its next visible size opens at latest.
      if (!history.current?.clientHeight) following.current = true;
      if (following.current) latest();
    });
    if (history.current) observer.observe(history.current);
    if (content.current) observer.observe(content.current);
    return () => observer.disconnect();
  }, [latest]);
  return (
    <section {...attributes} className={`conversation-window ${className}`}>
      <header className="conversation-window-bar thread-bar">
        <div className="conversation-heading">
          <h2 className="conversation-title" id={headingId}>
            {title}
          </h2>
          {caption && <span className="conversation-caption">{caption}</span>}
        </div>
        <span className="thread-bar-actions">{actions}</span>
      </header>
      {notice}
      <div className="conversation-history">
        <div
          className={`conversation-messages ${historyClassName}`}
          ref={history}
          role="region"
          aria-labelledby={headingId}
          tabIndex={-1}
          onScroll={() => {
            const node = history.current;
            if (!node) return;
            // Our queued scroll event can arrive after a resize. An unchanged
            // offset is not a reader leaving the bottom.
            if (node.scrollTop === position.current) return;
            position.current = node.scrollTop;
            following.current =
              node.scrollHeight - node.clientHeight - node.scrollTop <= bottomThreshold;
            if (following.current) clearPending();
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
      {composer && <div className="conversation-composer">{composer}</div>}
    </section>
  );
}
