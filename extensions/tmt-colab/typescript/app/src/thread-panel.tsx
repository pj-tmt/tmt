import { threadNeedsYou } from './comments-listing.js';
import { compactRelativeTime } from './display-time.js';
import type { ComposerConnectionPresentation } from './router.js';
import { AttachmentDropPanel } from './attachment-drop.js';
import type { CreatorTarget } from './creator-default.js';
import { Check, ChevronUp, CircleCheck, CircleDot, RotateCcw, X } from 'lucide-react';
import { createPortal } from 'react-dom';
import {
  BrowserAction,
  BrowserIconAction,
  BusyMark,
  busyMarkDelayMs,
  busyMarkHoldMs,
  useBusyMark,
} from '@tmt/browser-ui/react';
import { AgentList } from './components/agent-chip.js';
import { MessageComposer } from './components/message-composer.js';
import type { ComposerEdit } from './components/message-composer-edit.js';
import { captureConversation, conversationAsks } from './thread-store.js';
import { ConversationWindow } from './components/conversation-window.js';
import { ConversationPresentation } from './components/conversation-presentation.js';
import { conversationRecordKey } from './conversation-record-key.js';
import { threadNotificationDestination } from './reply-notification-context.js';
import { MarkdownText } from './components/markdown-text.js';
import { EchoAttachments, MessageAttachments } from './message-attachments.js';
import { ConversationTurn } from './components/conversation-turn.js';
import { useEffect, useState, type ReactNode } from 'react';
import { AskPanel, type AskRetryInput, type AskBinding, type PageAsk } from './ask-panel.js';
import { PageMenu as BrowserMenu } from './page-menu.js';
import type { ThreadBinding } from './thread-store.js';
import type { CommentView, QuoteSelector, ThreadView, DiscussionRef } from './thread-records.js';
import { isOwnDiscussionWriter, threadNumbers } from './thread-records.js';
import { text } from './strings.js';
import {
  echoLanded,
  MessageEchoContext,
  type MessageEcho,
  type MessageEchoHost,
} from './message-echo.js';
import { AnnotationInput, ArchivedDraft } from './annotation-input.js';
import {
  presentationOf,
  type ThreadPresentation,
  type ThreadStatusOutcome,
} from './thread-status-presentation.js';

/**
 * Host-owned echoes, in send order. A landed echo stays, and its verified record stays hidden,
 * until a mark that has shown has had its hold (`busyMarkHoldMs`); then the record takes its place.
 */
export function useMessageEcho(comments: readonly CommentView[], asks: readonly PageAsk[]) {
  const [echoes, setEchoes] = useState<readonly MessageEcho[]>([]);
  const [host] = useState<MessageEchoHost>(() => ({
    show: (echo) =>
      setEchoes((previous) =>
        previous.some((value) => value.local === echo.local)
          ? previous.map((value) => (value.local === echo.local ? echo : value))
          : [...previous, echo],
      ),
    remove: (local) => setEchoes((previous) => previous.filter((value) => value.local !== local)),
  }));
  const landed = echoes.filter((echo) => echoLanded(echo, comments, asks));
  const landedKey = landed.map((echo) => echo.local).join(' ');
  useEffect(() => {
    const timers = landed.map((echo) => {
      const now = Date.now();
      // A mark shows from busyMarkDelayMs after the echo; once shown it keeps its hold.
      const markShown = now - echo.at >= busyMarkDelayMs;
      const wait = markShown ? Math.max(0, echo.at + busyMarkDelayMs + busyMarkHoldMs - now) : 0;
      return setTimeout(() => {
        setEchoes((previous) => previous.filter((value) => value.local !== echo.local));
      }, wait);
    });
    return () => timers.forEach(clearTimeout);
  }, [landedKey]); // eslint-disable-line react-hooks/exhaustive-deps
  /** A verified record whose echo still holds its place is hidden until the swap. */
  const held = (comment: CommentView) =>
    echoes.some(
      (echo) =>
        echo.message?.writer === comment.ref.writer && echo.message.id === comment.messageId,
    );
  return [echoes, host, held] as const;
}

export function MessageEchoTurn({ echo, writer }: { echo: MessageEcho; writer?: string }) {
  const marked = useBusyMark(!echo.failure);
  return (
    <ConversationTurn
      role="user"
      author={text.askYou}
      at={echo.at}
      authorTitle={writer}
      className="comment"
      data-testid="comment-echo"
      data-echo={echo.failure ? 'failed' : 'pending'}
      // The verified record's message actions take this slot; holding it keeps the row's height.
      actions={<span className="comment-echo-actions" aria-hidden="true" />}
      meta={
        !echo.failure &&
        !echo.recipients.length && (
          // The mark has no words and follows the shared busy-mark delay.
          <span className="comment-echo-mark" aria-hidden="true" data-shown={marked || undefined}>
            {' '}
            <BusyMark />
          </span>
        )
      }
      status={
        !echo.failure && echo.recipients.length > 0 ? (
          <>
            {echo.recipients.map((recipient, index) => (
              <span key={index}>
                <span className="conversation-status" aria-label={`Ask ${recipient}`}>
                  <span className="ask-state" data-tone="waiting">
                    <span
                      className="comment-echo-mark"
                      aria-hidden="true"
                      data-shown={marked || undefined}
                    >
                      <BusyMark />
                    </span>
                  </span>
                  <span className="comment-echo-actions" aria-hidden="true" />
                </span>
              </span>
            ))}
          </>
        ) : undefined
      }
      delivery={
        echo.failure ? (
          // The row is the draft and the one place this failure is reported.
          <>
            <p role="alert" className="comment-echo-failure">
              <span className="annotation-send-mark" aria-hidden="true">
                ✗{' '}
              </span>
              {text.messageUnsent} {text.messageSendReference(echo.failure.reference)}
            </p>
            <div className="comment-actions">
              <BrowserAction
                type="button"
                variant="text"
                label={text.chatRetry}
                onActivate={(event) => {
                  if (event.isTrusted) echo.failure?.retry();
                }}
              />
              <BrowserAction
                type="button"
                variant="text"
                label={text.commentEdit}
                onActivate={(event) => {
                  if (event.isTrusted) echo.failure?.edit();
                }}
              />
            </div>
          </>
        ) : echo.recipients.length ? (
          <div />
        ) : undefined
      }
    >
      <MarkdownText className="comment-body" value={echo.body} names={echo.recipients} />
      <EchoAttachments attachments={echo.attachments} />
    </ConversationTurn>
  );
}

/** Actor names are admitted display metadata, never inferred from a device ID. */
export function resolvedLabel(status: ThreadPresentation['status'] | undefined) {
  return status?.actorName ? text.threadResolvedBy(status.actorName) : text.threadResolved;
}

/** The Discussion state line adds the signed status time without changing other surfaces. */
export function discussionResolvedLabel(
  status: ThreadPresentation['status'] | undefined,
  now = Date.now(),
) {
  const label = resolvedLabel(status);
  const at = status?.at === undefined ? NaN : Number(status.at);
  return Number.isFinite(at) ? `${label} · ${compactRelativeTime(at, now)}` : label;
}

export function DiscussionComment({
  comment,
  binding,
  blocked,
  allowEdit = true,
  readOnly = false,
  mentionedNames,
  status,
  delivery,
  attachments = true,
}: {
  comment: CommentView;
  allowEdit?: boolean;
  readOnly?: boolean;
  /** False where the same files already show elsewhere, such as a question's answer block. */
  attachments?: boolean;
  mentionedNames?: readonly string[];
  status?: ReactNode;
  delivery?: ReactNode;
  binding?: ThreadBinding;
  blocked: boolean;
}) {
  const [editing, setEditing] = useState(false),
    [draft, setDraft] = useState(comment.body),
    [editRevision, setEditRevision] = useState(comment.revision);
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(false);
  const owned = !readOnly && isOwnDiscussionWriter(comment.ref.writer, binding?.deviceId);
  const menu = [
    ...(owned && allowEdit && !comment.attachments?.length
      ? [{ key: 'edit', label: text.commentEdit, disabled: busy || blocked }]
      : []),
    ...(owned
      ? [
          {
            key: 'delete',
            label: 'Delete',
            disabled: busy || blocked,
          },
        ]
      : []),
  ];
  const action = (run: () => Promise<void>) => {
    if (readOnly || busy || blocked) return;
    setBusy(true);
    setError(false);
    void run()
      .then(
        () => setEditing(false),
        () => setError(true),
      )
      .finally(() => setBusy(false));
  };
  return (
    <ConversationTurn
      role="user"
      author={owned ? text.askYou : comment.deviceName || text.commentDevice}
      at={Number(comment.at)}
      authorTitle={comment.ref.writer}
      className="comment"
      data-testid="comment-entry"
      data-message-id={comment.messageId}
      data-conversation-record={conversationRecordKey(
        'comment',
        comment.ref.writer,
        comment.messageId,
      )}
      data-writer={comment.ref.writer}
      meta={!comment.deleted && comment.revision !== '1' && <> · {text.commentEdited}</>}
      actions={
        menu.length > 0 &&
        !comment.deleted &&
        !editing && (
          <BrowserMenu
            label="Message actions"
            items={menu}
            onSelect={(key) => {
              if (key === 'edit') {
                setDraft(comment.body);
                setEditRevision(comment.revision);
                setEditing(true);
              } else if (key === 'delete')
                action(() => binding!.deleteComment(comment.ref, comment.revision));
            }}
          />
        )
      }
      status={status}
      delivery={
        <>
          {!busy && delivery}
          {error && <p role="alert">{text.commentFailed}</p>}
        </>
      }
    >
      {comment.deleted ? (
        <p className="comment-status">{text.commentDeleted}</p>
      ) : (
        <>
          {editing && readOnly ? (
            <ArchivedDraft value={draft} />
          ) : editing ? (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                if (event.isTrusted && draft.trim())
                  action(() => binding!.edit(comment.ref, editRevision, draft));
              }}
            >
              <MessageComposer
                label={text.commentEditBody}
                edit={{ value: draft }}
                disabled={busy || blocked}
                busy={busy && !blocked}
                autoFocus
                onChange={(next) => setDraft(next.value)}
              />
              <div className="comment-actions">
                <BrowserAction
                  type="submit"
                  variant="primary"
                  label={text.commentSave}
                  busy={busy}
                  busyMark={<BusyMark />}
                  disabled={blocked || !draft.trim()}
                  onActivate={() => {}}
                />
                <BrowserAction
                  type="button"
                  variant="text"
                  label={text.commentCancel}
                  busy={busy}
                  onActivate={(event) => {
                    if (event.isTrusted) setEditing(false);
                  }}
                />
              </div>
            </form>
          ) : (
            <>
              <MarkdownText className="comment-body" value={comment.body} names={mentionedNames} />
              {attachments && (
                <MessageAttachments comment={comment} binding={binding?.attachments} />
              )}
            </>
          )}
        </>
      )}
    </ConversationTurn>
  );
}

/** One association owner for both annotation and Chat turns. */
export function CommentExchange({
  comment,
  thread,
  binding,
  ask,
  asks,
  blocked,
  allowEdit = true,
  readOnly = false,
  attachments = true,
  title,
  retryInput,
}: {
  comment: CommentView;
  thread: ThreadView;
  binding?: ThreadBinding;
  ask?: AskBinding;
  asks: readonly PageAsk[];
  blocked: boolean;
  allowEdit?: boolean;
  readOnly?: boolean;
  /** False where the comment's files already show elsewhere. */
  attachments?: boolean;
  title: string;
  retryInput?: AskRetryInput;
}) {
  const records = conversationAsks(thread, asks, true).filter((record) =>
    record.messageIds?.includes(comment.messageId),
  );
  const ordinaryRetryInput =
    !thread.deleted &&
    !comment.deleted &&
    isOwnDiscussionWriter(comment.ref.writer, binding?.deviceId)
      ? {
          quote: thread.anchor?.exact ?? '',
          comment: comment.body,
          title,
          url: location.href,
          context: {
            thread: thread.ref,
            message: comment.ref,
            threadRevision: thread.revision,
            messageRevision: comment.revision,
            conversation: captureConversation(
              {
                ...thread,
                comments: thread.comments.slice(0, thread.comments.indexOf(comment)),
              },
              asks,
            ),
          },
        }
      : undefined;
  const user = (status?: ReactNode, delivery?: ReactNode) => (
    <DiscussionComment
      readOnly={readOnly}
      comment={comment}
      binding={binding}
      blocked={blocked}
      allowEdit={allowEdit}
      attachments={attachments}
      mentionedNames={records.map((record) => record.agentName)}
      status={status}
      delivery={delivery}
    />
  );
  return records.length ? (
    <AskPanel
      readOnly={readOnly}
      inline
      records={records}
      binding={ask}
      blocked={blocked}
      renderUser={(_record, status, delivery) => user(status, delivery)}
      retryInput={
        typeof retryInput === 'function'
          ? (record) =>
              retryInput(record) ??
              ((record.messageIds?.length ?? 0) < 2 ? ordinaryRetryInput : undefined)
          : (retryInput ?? ordinaryRetryInput)
      }
    />
  ) : (
    user()
  );
}

export type ThreadWindowProps = {
  number?: number;
  creationRecipient?: CreatorTarget;
  /** Absent until the first committed turn; the composer stays in this same window. */
  thread?: ThreadView;
  anchor?: QuoteSelector;
  layout?: 'panel' | 'anchored' | 'discussion';
  attached: boolean;
  anchorsChecked: boolean;
  selection: QuoteSelector | null;
  binding?: ThreadBinding;
  ask?: AskBinding;
  title: string;
  asks: readonly PageAsk[];
  close(): void;
  blocked: boolean;
  connectionPresentation?: ComposerConnectionPresentation;
  observationUnavailable?: boolean;
  archived?: boolean;
  composer?: ReactNode;
  initialEdit?: ComposerEdit;
  onDraft?(edit: ComposerEdit): void;
  onBusy?(busy: boolean): void;
  /** Presentation inputs for this thread, computed once by the authenticated parent. */
  status?: ThreadPresentation['status'];
  /** The parent owns recipient freezing and notification; the window only asks. */
  onStatusChange?(resolved: boolean): Promise<ThreadStatusOutcome>;
  /**
   * The status a change shows before its record lands. A parent sharing the thread across
   * presentations owns it; otherwise the window keeps its own.
   */
  pendingResolved?: boolean;
  onPendingResolved?(pending: boolean | undefined): void;
  /** The discussion layout's anchor: show the thread's passage in the page. */
  onShowInPage?(): void;
  /** The discussion layout's Resolve or Reopen stands in the panel's header, in this slot. */
  headerSlot?: HTMLElement | null;
};

export function ThreadWindow({
  number,
  thread,
  anchor: initialAnchor,
  creationRecipient,
  layout = 'panel',
  attached,
  anchorsChecked,
  selection,
  binding,
  ask,
  title,
  asks,
  close,
  blocked,
  connectionPresentation,
  observationUnavailable,
  archived = false,
  composer,
  initialEdit,
  onDraft,
  onBusy,
  status,
  onStatusChange,
  pendingResolved: parentPending,
  onPendingResolved,
  onShowInPage,
  headerSlot,
}: ThreadWindowProps) {
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(false),
    [notNotified, setNotNotified] = useState<string[]>([]);
  const [showResolved, setShowResolved] = useState(false);
  // A status change shows in the click frame; the verified record replaces it, or a failure undoes it.
  const [ownPending, setOwnPending] = useState<boolean>();
  const pendingResolved = onPendingResolved ? parentPending : ownPending;
  const setPendingResolved = onPendingResolved ?? setOwnPending;
  const [echoes, echoHost, held] = useMessageEcho(thread?.comments ?? [], asks);
  const shownResolved = pendingResolved ?? thread?.resolved;
  useEffect(() => setShowResolved(false), [thread?.ref.writer, thread?.threadId, thread?.resolved]);
  useEffect(() => {
    if (pendingResolved === thread?.resolved) setPendingResolved(undefined);
  }, [pendingResolved, thread?.resolved, setPendingResolved]);
  const discussion = layout === 'discussion';
  const collapsed = !discussion && !!thread && !!shownResolved && !showResolved;
  const owned = !!thread && isOwnDiscussionWriter(thread.ref.writer, binding?.deviceId);
  const anchor = thread ? thread.anchor : initialAnchor;
  // A resolved thread leaves the page's marker set, so its attachment is not
  // observed until Reopen restores the marker.
  const tracked = !!anchor && !shownResolved;
  const messageIds = [
    ...echoes.map((echo) => `echo:${echo.local}`),
    ...(thread?.comments
      .filter((comment) => !held(comment))
      .map((comment) => conversationRecordKey('comment', comment.ref.writer, comment.messageId)) ??
      []),
    ...conversationAsks(thread, asks, true)
      .filter((record) => record.reply !== undefined)
      .map((record) => conversationRecordKey('reply', record.writer, record.operationId)),
  ];
  const [reattach, setReattach] = useState<QuoteSelector | null>(null);
  const action = (change: Parameters<ThreadBinding['updateThread']>[2]) => {
    if (!thread || !binding || busy || blocked) return;
    setBusy(true);
    onBusy?.(true);
    setError(false);
    void Promise.resolve()
      .then(() => binding.updateThread(thread.ref, thread.revision, change))
      .then(
        () => setReattach(null),
        () => setError(true),
      )
      .finally(() => {
        setBusy(false);
        onBusy?.(false);
      });
  };
  const changeStatus = (resolved: boolean) => {
    if (!thread || !onStatusChange || busy || blocked) return;
    setBusy(true);
    onBusy?.(true);
    setError(false);
    setNotNotified([]);
    setPendingResolved(resolved);
    let closeAfter = false;
    void Promise.resolve()
      .then(() => onStatusChange(resolved))
      .then(
        (result) => {
          const names = (result.warnings ?? []).map((warning) => warning.name);
          setReattach(null);
          setNotNotified(names);
          // The record that follows replaces the display; nothing changed means nothing follows.
          if (!result.changed) setPendingResolved(undefined);
          // Keep the window open while it still has something to tell the person.
          closeAfter = result.changed && resolved && layout === 'anchored' && !names.length;
        },
        () => {
          setPendingResolved(undefined);
          setError(true);
        },
      )
      .finally(() => {
        setBusy(false);
        onBusy?.(false);
        if (closeAfter) close();
      });
  };
  // The Discussion view's Resolve, or Reopen, stands in the panel's header; the window keeps its
  // state, its busy fence and its failure line, and only draws the control there.
  const resolveAction =
    discussion &&
    headerSlot &&
    thread &&
    !archived &&
    onStatusChange &&
    status?.controllable &&
    !thread.deleted
      ? createPortal(
          <BrowserIconAction
            type="button"
            variant="text"
            label={
              pendingResolved !== undefined
                ? pendingResolved
                  ? text.threadResolving
                  : text.threadReopening
                : thread.resolved
                  ? text.threadReopen
                  : text.threadResolve
            }
            disabled={blocked}
            busy={busy}
            icon={thread.resolved ? <RotateCcw /> : <Check />}
            onActivate={(event) => {
              if (event.isTrusted) changeStatus(!thread.resolved);
            }}
          />,
          headerSlot,
        )
      : null;
  return (
    <AttachmentDropPanel target="thread" active={!collapsed}>
      <ConversationWindow
        collapsed={collapsed}
        hideHeader={discussion}
        historyLabel={discussion ? text.discussion : undefined}
        summary={
          <div className="resolved-summary">
            <p>
              {text.threadMessageCount(messageIds.length)} ·{' '}
              {thread?.comments
                .filter((comment) => !comment.deleted)
                .reduce((count, comment) => count + (comment.attachments?.length ?? 0), 0) ??
                0}{' '}
              attachments
            </p>
            <BrowserAction
              type="button"
              variant="text"
              label="Show conversation"
              onActivate={(event) => {
                if (event.isTrusted) setShowResolved(true);
              }}
            />
          </div>
        }
        notificationDestination={
          thread ? threadNotificationDestination(thread.ref.writer, thread.ref.id) : undefined
        }
        className="comment-thread"
        data-layout={layout}
        data-testid={thread ? 'comment-thread' : 'annotation-window'}
        data-thread-id={thread?.threadId}
        data-writer={thread?.ref.writer}
        data-anchor={
          thread?.deleted || !anchor
            ? 'page'
            : shownResolved
              ? 'resolved'
              : attached
                ? 'attached'
                : 'detached'
        }
        title={
          <span className="thread-state" title={anchor?.exact}>
            {number !== undefined && <span className="annotation-number">{number}</span>}
            {thread && <span className="annotation-heading">{text.threadHeading}</span>}
            {thread && (shownResolved ? <CircleCheck aria-hidden /> : <CircleDot aria-hidden />)}
            <span>
              {!thread
                ? text.annotateSelection
                : thread.deleted
                  ? text.threadDeleted
                  : shownResolved
                    ? pendingResolved
                      ? text.threadResolving
                      : resolvedLabel(status)
                    : text.threadOpen}
            </span>
          </span>
        }
        actions={
          <>
            {tracked && thread && (
              <span className="comment-status">
                {attached ? text.commentAnchored : text.commentDetached}
              </span>
            )}

            {!archived && thread && onStatusChange && status?.controllable && !thread.deleted && (
              <BrowserIconAction
                type="button"
                variant="text"
                disabled={blocked}
                busy={busy}
                label={
                  pendingResolved !== undefined
                    ? pendingResolved
                      ? text.threadResolving
                      : text.threadReopening
                    : thread.resolved
                      ? text.threadReopen
                      : layout === 'anchored'
                        ? text.threadResolveAction
                        : text.threadResolve
                }
                icon={thread.resolved ? <RotateCcw /> : <CircleCheck />}
                onActivate={(event) => {
                  if (event.isTrusted) changeStatus(!thread.resolved);
                }}
              />
            )}
            <BrowserIconAction
              type="button"
              variant="text"
              label={
                layout === 'anchored'
                  ? text.threadCollapse
                  : thread
                    ? text.threadClose
                    : text.annotationClose
              }
              tooltipLabel={layout === 'anchored' ? text.threadCollapseHelp : undefined}
              ariaKeyShortcuts={layout === 'anchored' ? 'Escape' : undefined}
              icon={layout === 'anchored' ? <ChevronUp /> : <X />}
              busy={busy}
              onActivate={(event) => {
                if (event.isTrusted) close();
              }}
            />
          </>
        }
        notice={
          <>
            {discussion && thread && (
              <>
                {anchor &&
                  (onShowInPage ? (
                    // The thread's title: its number and the passage it is about; activating it shows
                    // the passage in the page.
                    <button
                      type="button"
                      className="discussion-quote thread-state"
                      title={anchor.exact}
                      aria-label={`${text.threadShowInPage}: ${anchor.exact}`}
                      onClick={(event) => {
                        if (event.isTrusted) onShowInPage();
                      }}
                    >
                      {number !== undefined && <span className="annotation-number">{number}</span>}
                      <q>{anchor.exact}</q>
                    </button>
                  ) : (
                    <div className="discussion-quote thread-state" title={anchor.exact}>
                      {number !== undefined && <span className="annotation-number">{number}</span>}
                      <q>{anchor.exact}</q>
                    </div>
                  ))}
                {anchor && (
                  <span
                    className="comment-status"
                    style={{ visibility: tracked && !attached ? 'visible' : 'hidden' }}
                    aria-hidden={!tracked || attached}
                  >
                    {text.commentDetached}
                  </span>
                )}
              </>
            )}
            {layout === 'anchored' && anchor && (
              <blockquote className="annotation-quote" title={anchor.exact}>
                <q>{anchor.exact}</q>
              </blockquote>
            )}
            {observationUnavailable && (
              <p role="status" className="annotation-hint">
                {text.askObservationUnavailable}
              </p>
            )}
            {notNotified.length > 0 && (
              <p role="status" className="annotation-hint">
                {text.threadNotNotified} <AgentList names={notNotified} />
              </p>
            )}
            {error && <p role="alert">{text.commentFailed}</p>}
          </>
        }
        messageIds={messageIds}
        ownWriter={binding?.deviceId}
        historyClassName="thread-messages"
        composer={
          // One echo seam for the built-in composer and a parent's (the anchored dialog).
          <MessageEchoContext value={echoHost}>
            {composer !== undefined
              ? composer
              : thread &&
                !thread.deleted &&
                binding && (
                  <ConversationPresentation
                    value={layout === 'discussion' ? 'discussion' : 'thread'}
                  >
                    <AnnotationInput
                      connectionPresentation={connectionPresentation}
                      recoveryRequired={connectionPresentation?.phase === 'reconnecting'}
                      archived={archived}
                      creationRecipient={creationRecipient}
                      binding={ask}
                      discussion={binding}
                      anchor={thread.anchor}
                      thread={thread}
                      asks={asks}
                      title={title}
                      blocked={blocked || busy}
                      initialEdit={initialEdit}
                      onDraft={(_value, edit) => onDraft?.(edit)}
                      onBusy={onBusy}
                      cancel={close}
                      committed={() => {}}
                    />
                  </ConversationPresentation>
                )}
          </MessageEchoContext>
        }
      >
        {resolveAction}
        {tracked && anchorsChecked && !attached && (
          <p className="annotation-hint">{text.commentQuoteChanged}</p>
        )}
        {thread?.comments
          .filter((comment) => !held(comment))
          .map((comment) => (
            <CommentExchange
              key={`${comment.ref.writer}:${comment.messageId}`}
              readOnly={archived}
              allowEdit={!archived}
              comment={comment}
              thread={thread}
              binding={binding}
              ask={ask}
              asks={asks}
              blocked={blocked || busy}
              title={title}
            />
          ))}
        {echoes.map((echo) => (
          <MessageEchoTurn key={echo.local} echo={echo} writer={binding?.deviceId} />
        ))}
        {/* A thread just created here is not attached until the page has checked its anchors: the
            row waits for that, rather than flashing in for a frame and moving the window. */}
        {!archived &&
          binding &&
          thread &&
          !thread.proposal &&
          !thread.deleted &&
          owned &&
          tracked &&
          anchorsChecked &&
          !attached && (
            <div className="comment-actions">
              <BrowserAction
                type="button"
                variant="text"
                label={text.commentReattach}
                busy={busy}
                disabled={blocked || !selection}
                onActivate={(event) => {
                  if (event.isTrusted && selection) setReattach(structuredClone(selection));
                }}
              />
            </div>
          )}
        {!archived && reattach && thread && !thread.deleted && (
          <section className="comment-compose">
            <blockquote>{reattach.exact}</blockquote>
            <div className="comment-actions">
              <BrowserAction
                type="button"
                variant="text"
                label={text.commentConfirmReattach}
                disabled={blocked}
                busy={busy}
                onActivate={(event) => {
                  if (event.isTrusted) action({ anchor: reattach });
                }}
              />
              <BrowserAction
                type="button"
                variant="text"
                label={text.commentCancel}
                busy={busy}
                onActivate={(event) => {
                  if (event.isTrusted) setReattach(null);
                }}
              />
            </div>
          </section>
        )}
      </ConversationWindow>
    </AttachmentDropPanel>
  );
}

export function ThreadPanel({
  numbers: suppliedNumbers,
  creationRecipient,
  hideHeader = false,
  threads,
  resolved,
  anchorsChecked,
  selection,
  binding,
  ask,
  asks,
  title,
  blocked,
  connectionPresentation,
  archived = false,
  active,
  close,
  draft,
  onDraft,
  presentations,
  onStatusChange,
  onBusy,
  renderProposal,
  proposalStatusPending,
  showInPage,
  headerSlot,
  batchStatus,
}: {
  creationRecipient?: CreatorTarget;
  hideHeader?: boolean;
  threads: readonly ThreadView[];
  numbers?: ReadonlyMap<string, number>;
  resolved: readonly string[];
  anchorsChecked: boolean;
  selection: QuoteSelector | null;
  binding?: ThreadBinding;
  ask?: AskBinding;
  asks: readonly PageAsk[];
  title: string;
  blocked: boolean;
  connectionPresentation?: ComposerConnectionPresentation;
  archived?: boolean;
  active: string | null;
  close(): void;
  draft?(ref: DiscussionRef): ComposerEdit | undefined;
  onDraft?(ref: DiscussionRef, edit: ComposerEdit): void;
  presentations?: readonly ThreadPresentation[];
  onStatusChange?(thread: ThreadView, resolved: boolean): Promise<ThreadStatusOutcome>;
  onBusy?(busy: boolean): void;
  renderProposal?(thread: ThreadView): ReactNode;
  proposalStatusPending?: ReadonlyMap<string, boolean>;
  /** The Discussion view's anchor. */
  showInPage?(ref: DiscussionRef): void;
  /** Where the Discussion view puts its Resolve control: the Comments panel's header. */
  headerSlot?: HTMLElement | null;
  batchStatus?: ReactNode;
}) {
  const numbers = suppliedNumbers ?? threadNumbers(threads);
  const [pending, setPending] = useState<Record<string, boolean>>({});
  const discussed = threads.find(
    (thread) => !thread.deleted && `${thread.ref.writer}:${thread.threadId}` === active,
  );
  if (discussed) {
    const thread = discussed;
    const id = `${thread.ref.writer}:${thread.threadId}`;
    const status = presentationOf(presentations, thread.ref)?.status;
    const pendingResolved = thread.proposal ? proposalStatusPending?.get(id) : pending[id];
    const shownResolved = pendingResolved ?? thread.resolved;
    const needsYou = threadNeedsYou({ ...thread, resolved: shownResolved }, presentations);
    return (
      <aside
        className="comments-panel"
        aria-label={text.discussion}
        data-testid="comments-panel"
        data-view="discussion"
        data-thread-id={thread.threadId}
      >
        {/* The reserved state line keeps the quote and composer still through Resolve. */}
        <div className="discussion-status">
          {batchStatus && <span role="status">{batchStatus}</span>}
          {!batchStatus && (shownResolved || needsYou) && (
            <span
              className="discussion-state"
              data-state={shownResolved ? 'resolved' : 'needs'}
              data-testid="discussion-state"
            >
              {shownResolved ? <Check aria-hidden /> : <span aria-hidden="true">◆</span>}
              {shownResolved
                ? pendingResolved !== undefined
                  ? text.threadResolving
                  : discussionResolvedLabel(status)
                : text.needsYouWord}
            </span>
          )}
        </div>
        {thread.proposal ? (
          renderProposal?.(thread)
        ) : (
          <ConversationPresentation value="discussion">
            <ThreadWindow
              key={id}
              layout="discussion"
              connectionPresentation={connectionPresentation}
              archived={archived}
              creationRecipient={creationRecipient}
              thread={thread}
              number={numbers.get(id)}
              attached={resolved.includes(id)}
              anchorsChecked={anchorsChecked}
              selection={selection}
              binding={binding}
              ask={ask}
              asks={asks}
              title={title}
              close={close}
              blocked={blocked}
              initialEdit={draft?.(thread.ref)}
              onDraft={(edit) => onDraft?.(thread.ref, edit)}
              onBusy={onBusy}
              status={status}
              onStatusChange={onStatusChange && ((resolved) => onStatusChange(thread, resolved))}
              pendingResolved={pending[id]}
              onPendingResolved={(value) =>
                setPending((previous) => {
                  const { [id]: _, ...rest } = previous;
                  return value === undefined ? rest : { ...rest, [id]: value };
                })
              }
              onShowInPage={showInPage && thread.anchor ? () => showInPage(thread.ref) : undefined}
              headerSlot={headerSlot}
            />
          </ConversationPresentation>
        )}
      </aside>
    );
  }
  return (
    <aside className="comments-panel" aria-label={text.comments} data-testid="comments-panel">
      {!hideHeader && <h2>{text.comments}</h2>}
      <div className="comments-empty" data-testid="comments-empty">
        <Check aria-hidden />
        <h3>{threads.length ? text.commentsEmptyTitle : text.commentsEmptyNone}</h3>
        <p>{threads.length ? text.commentsEmptyCaughtUp : text.commentsEmptyNoneHint}</p>
      </div>
    </aside>
  );
}
