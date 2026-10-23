import type { ComposerConnectionPresentation } from './router.js';
import { AttachmentDropPanel } from './attachment-drop.js';
import type { CreationRecipient } from './fold-protocol.js';
import { ArrowLeft, Check, ChevronUp, CircleCheck, CircleDot, RotateCcw, X } from 'lucide-react';
import {
  BrowserAction,
  BrowserIconAction,
  BrowserTabs,
  type BrowserTab,
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
import { conversationRecordKey } from './conversation-record-key.js';
import { threadNotificationDestination } from './reply-notification-context.js';
import { MarkdownText } from './components/markdown-text.js';
import { EchoAttachments, MessageAttachments } from './message-attachments.js';
import { ConversationTurn } from './components/conversation-turn.js';
import { useEffect, useId, useState, type ReactNode } from 'react';
import { AskPanel, type AskRetryInput, type AskBinding, type PageAsk } from './ask-panel.js';
import { PageMenu as BrowserMenu } from './page-menu.js';
import type { ThreadBinding } from './thread-store.js';
import type { CommentView, QuoteSelector, ThreadView, DiscussionRef } from './thread-records.js';
import { isOwnDiscussionWriter, threadNumbers } from './thread-records.js';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
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
export function useMessageEcho(comments: readonly CommentView[]) {
  const [echoes, setEchoes] = useState<readonly MessageEcho[]>([]);
  const [released, setReleased] = useState<ReadonlySet<string>>(() => new Set());
  const [host] = useState<MessageEchoHost>(() => ({
    show: (echo) =>
      setEchoes((previous) =>
        previous.some((value) => value.local === echo.local)
          ? previous.map((value) => (value.local === echo.local ? echo : value))
          : [...previous, echo],
      ),
    remove: (local) => setEchoes((previous) => previous.filter((value) => value.local !== local)),
  }));
  const landed = echoes.filter((echo) => echoLanded(echo, comments));
  const landedKey = landed.map((echo) => echo.local).join(' ');
  useEffect(() => {
    const timers = landed.map((echo) => {
      const now = Date.now();
      // A mark shows from busyMarkDelayMs after the echo; once shown it keeps its hold.
      const markShown = now - echo.at >= busyMarkDelayMs;
      const wait = markShown ? Math.max(0, echo.at + busyMarkDelayMs + busyMarkHoldMs - now) : 0;
      return setTimeout(() => {
        setEchoes((previous) => previous.filter((value) => value.local !== echo.local));
        setReleased((previous) => new Set(previous).add(echo.local));
      }, wait);
    });
    return () => timers.forEach(clearTimeout);
  }, [landedKey]); // eslint-disable-line react-hooks/exhaustive-deps
  /** A verified record whose echo still holds its place is hidden until the swap. */
  const held = (comment: CommentView) =>
    echoes.some(
      (echo) =>
        !released.has(echo.local) &&
        echo.message?.writer === comment.ref.writer &&
        echo.message.id === comment.messageId,
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
        !echo.failure && (
          // The mark has no words and follows the shared busy-mark delay.
          <span className="comment-echo-mark" aria-hidden="true" data-shown={marked || undefined}>
            {' '}
            <BusyMark />
          </span>
        )
      }
      delivery={
        echo.failure && (
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
        )
      }
    >
      <MarkdownText className="comment-body" value={echo.body} names={[]} />
      <EchoAttachments attachments={echo.attachments} />
    </ConversationTurn>
  );
}

/** Actor names are admitted display metadata, never inferred from a device ID. */
export function resolvedLabel(status: ThreadPresentation['status'] | undefined) {
  return status?.actorName ? text.threadResolvedBy(status.actorName) : text.threadResolved;
}

function Composer({
  label,
  quote,
  submit,
  cancel,
  blocked,
  archived = false,
}: {
  label: string;
  quote?: string;
  submit(body: string): Promise<void>;
  cancel?(): void;
  blocked: boolean;
  archived?: boolean;
}) {
  const [resetKey, setResetKey] = useState(0);
  const [body, setBody] = useState(''),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(false);
  if (archived) return <ArchivedDraft value={body} />;
  return (
    <form
      className="comment-compose"
      onSubmit={(event) => {
        event.preventDefault();
        if (!event.isTrusted || busy || blocked || !body.trim()) return;
        setBusy(true);
        setError(false);
        void submit(body)
          .then(
            () => {
              setBody('');
              setResetKey((key) => key + 1);
              cancel?.();
            },
            () => setError(true),
          )
          .finally(() => setBusy(false));
      }}
    >
      {quote && <blockquote>{quote}</blockquote>}
      <p>{label}</p>
      <MessageComposer
        label={label}
        edit={{ value: body }}
        resetKey={resetKey}
        disabled={busy || blocked}
        busy={busy && !blocked}
        onChange={(next) => setBody(next.value)}
      />
      <div className="comment-actions">
        <BrowserAction
          type="submit"
          variant="primary"
          label={label}
          busy={busy}
          busyMark={<BusyMark />}
          disabled={blocked || !body.trim()}
          onActivate={() => {}}
        />
        {cancel && (
          <BrowserAction
            type="button"
            variant="text"
            label={text.commentCancel}
            busy={busy}
            onActivate={(event) => {
              if (event.isTrusted) cancel();
            }}
          />
        )}
      </div>
      {error && <p role="alert">{text.commentFailed}</p>}
    </form>
  );
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

/** The thread's title: a proposal's title, else the first line of its first comment. */
export function threadTitle(thread: ThreadView) {
  if (thread.deleted) return text.threadDeleted;
  if (thread.proposal) return thread.proposal.title;
  const first =
    thread.comments.find((value) => !value.deleted && value.ref.writer === thread.ref.writer) ??
    thread.comments.find((value) => !value.deleted);
  return first?.body.split('\n')[0] || text.pageComments;
}

export type ThreadWindowProps = {
  number?: number;
  creationRecipient?: CreationRecipient;
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
   * The status a change shows before its record lands. A parent that also renders the thread's
   * row owns it, so the row and the window agree; without one the window keeps its own.
   */
  pendingResolved?: boolean;
  onPendingResolved?(pending: boolean | undefined): void;
  /** The discussion layout's passage arrow: show the thread's passage in the page. */
  onShowInPage?(): void;
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
}: ThreadWindowProps) {
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(false),
    [notNotified, setNotNotified] = useState<string[]>([]);
  const [showResolved, setShowResolved] = useState(false);
  // A status change shows in the click frame; the verified record replaces it, or a failure undoes it.
  const [ownPending, setOwnPending] = useState<boolean>();
  const pendingResolved = onPendingResolved ? parentPending : ownPending;
  const setPendingResolved = onPendingResolved ?? setOwnPending;
  const [echoes, echoHost, held] = useMessageEcho(thread?.comments ?? []);
  const shownResolved = pendingResolved ?? thread?.resolved;
  useEffect(() => setShowResolved(false), [thread?.ref.writer, thread?.threadId, thread?.resolved]);
  useEffect(() => {
    if (pendingResolved === thread?.resolved) setPendingResolved(undefined);
  }, [pendingResolved, thread?.resolved, setPendingResolved]);
  const collapsed = !!thread && !!shownResolved && !showResolved;
  const discussion = layout === 'discussion';
  const owned = !!thread && isOwnDiscussionWriter(thread.ref.writer, binding?.deviceId);
  const anchor = thread ? thread.anchor : initialAnchor;
  // A resolved thread leaves the page's marker set, so its attachment is not
  // observed until Reopen restores the marker.
  const tracked = !!anchor && !shownResolved;
  const messageIds = [
    ...(thread?.comments.map((comment) =>
      conversationRecordKey('comment', comment.ref.writer, comment.messageId),
    ) ?? []),
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
            {thread && <span className="annotation-heading">THREAD</span>}
            {thread && (shownResolved ? <CircleCheck aria-hidden /> : <CircleDot aria-hidden />)}
            <span>
              {!thread
                ? 'ANNOTATE THE SELECTION'
                : thread.deleted
                  ? text.threadDeleted
                  : shownResolved
                    ? pendingResolved
                      ? text.threadResolved
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
                  thread.resolved
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
                <h3 className="discussion-title">{threadTitle(thread)}</h3>
                {anchor && (
                  <div className="discussion-quote thread-state" title={anchor.exact}>
                    {number !== undefined && <span className="annotation-number">{number}</span>}
                    <q>{anchor.exact}</q>
                    {onShowInPage && (
                      <BrowserIconAction
                        type="button"
                        variant="text"
                        label={text.threadShowInPage}
                        icon={<ArrowLeft />}
                        onActivate={(event) => {
                          if (event.isTrusted) onShowInPage();
                        }}
                      />
                    )}
                  </div>
                )}
                {tracked && (
                  <span className="comment-status">
                    {attached ? text.commentAnchored : text.commentDetached}
                  </span>
                )}
              </>
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
        footer={
          discussion &&
          thread &&
          !archived &&
          onStatusChange &&
          status?.controllable &&
          !thread.deleted ? (
            <div className="discussion-footer">
              {/* A quiet action: the shared action with a mark, not underlined link text. */}
              <button
                type="button"
                className={ui.action}
                data-variant="text"
                disabled={blocked}
                aria-disabled={busy || undefined}
                aria-busy={busy || undefined}
                onClick={(event) => {
                  if (busy) event.preventDefault();
                  else if (event.isTrusted) changeStatus(!thread.resolved);
                }}
              >
                {thread.resolved ? <RotateCcw aria-hidden /> : <CircleCheck aria-hidden />}
                <span className={ui.actionLabel}>
                  {thread.resolved ? text.threadReopen : text.threadResolve}
                </span>
              </button>
            </div>
          ) : undefined
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
                )}
          </MessageEchoContext>
        }
      >
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

type CommentsView = 'open' | 'needs' | 'resolved' | 'proposals';

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
  select,
  draft,
  onDraft,
  presentations,
  onStatusChange,
  onBusy,
  renderProposal,
  reopenProposal,
  showInPage,
}: {
  creationRecipient?: CreationRecipient;
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
  select(ref: DiscussionRef | null): void;
  draft?(ref: DiscussionRef): ComposerEdit | undefined;
  onDraft?(ref: DiscussionRef, edit: ComposerEdit): void;
  presentations?: readonly ThreadPresentation[];
  onStatusChange?(thread: ThreadView, resolved: boolean): Promise<ThreadStatusOutcome>;
  onBusy?(busy: boolean): void;
  renderProposal?(thread: ThreadView): ReactNode;
  reopenProposal?(thread: ThreadView): void;
  /** The Discussion view's passage arrow. */
  showInPage?(ref: DiscussionRef): void;
}) {
  const numbers = suppliedNumbers ?? threadNumbers(threads);
  const [compose, setCompose] = useState(false);
  const [view, setView] = useState<CommentsView>('open');
  const [pending, setPending] = useState<Record<string, boolean>>({});
  const scope = useId();
  const hasProposals =
    !!renderProposal && threads.some((thread) => thread.proposal && !thread.deleted);
  const shownView = view === 'proposals' && !hasProposals ? 'open' : view;
  const rows = threads.map((thread) => {
    const id = `${thread.ref.writer}:${thread.threadId}`;
    const status = presentationOf(presentations, thread.ref)?.status;
    // A status change shows in the row in the same frame as in its window.
    const shownResolved = pending[id] ?? thread.resolved;
    // Waiting for the person: a proposal with no decision, or news they have not seen.
    const needsYou =
      !shownResolved &&
      !thread.deleted &&
      ((!!thread.proposal && (!thread.decision || thread.decision.deleted)) || !!status?.unseen);
    return { thread, id, status, shownResolved, needsYou };
  });
  const matches = (row: (typeof rows)[number]) =>
    shownView === 'open'
      ? !row.shownResolved
      : shownView === 'needs'
        ? row.needsYou
        : shownView === 'resolved'
          ? row.shownResolved
          : !!row.thread.proposal;
  const visibleRows = rows.filter(matches);
  const discussed = active ? rows.find((row) => row.id === active) : undefined;
  const tab = (id: CommentsView, label: string, state?: BrowserTab['state']): BrowserTab => ({
    id,
    tabId: `${scope}-tab-${id}`,
    panelId: `${scope}-panel-${id}`,
    label,
    ...(state ? { state } : {}),
  });
  // News in a resolved thread is not in the Open view; its tab says so.
  const resolvedNews = rows.some((row) => row.shownResolved && row.status?.unseen);
  const tabs = [
    tab('open', text.commentsViewOpen),
    tab('needs', text.needsYouWord),
    tab(
      'resolved',
      text.threadResolved,
      resolvedNews ? { mark: '●', tone: 'waiting', word: text.threadUnseen } : undefined,
    ),
    ...(hasProposals ? [tab('proposals', text.commentsViewProposals)] : []),
  ];
  if (discussed) {
    const { thread, id, status, shownResolved, needsYou } = discussed;
    return (
      <aside
        className="comments-panel"
        aria-label={text.discussion}
        data-testid="comments-panel"
        data-view="discussion"
      >
        <div className="discussion-status">
          <span
            className="discussion-state"
            data-state={shownResolved ? 'resolved' : needsYou ? 'needs' : 'open'}
            data-testid="discussion-state"
          >
            {shownResolved ? (
              <Check aria-hidden />
            ) : needsYou ? (
              <span aria-hidden="true">◆</span>
            ) : null}
            {shownResolved
              ? pending[id] !== undefined
                ? text.threadResolved
                : resolvedLabel(status)
              : needsYou
                ? text.needsYouWord
                : text.discussionOpen}
          </span>
          <BrowserAction
            type="button"
            variant="text"
            label={text.allComments}
            onActivate={(event) => {
              if (event.isTrusted) select(null);
            }}
          />
        </div>
        {thread.proposal ? (
          renderProposal?.(thread)
        ) : (
          <ThreadWindow
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
            close={() => select(null)}
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
          />
        )}
      </aside>
    );
  }
  return (
    <aside className="comments-panel" aria-label={text.comments} data-testid="comments-panel">
      {!hideHeader && (
        <header>
          <h2>{text.comments}</h2>
          <span>{threads.filter((value) => !value.deleted).length}</span>
        </header>
      )}
      <BrowserTabs
        label={text.commentsViews}
        variant="flat"
        wrap
        tabs={tabs}
        selected={shownView}
        onSelect={(id) => setView(id as CommentsView)}
      >
        {(current) =>
          current.id !== shownView ? null : (
            <>
              {!archived && (
                <div className="comments-add">
                  <BrowserAction
                    type="button"
                    variant="text"
                    label={`+ ${text.commentPage}`}
                    disabled={!binding || blocked || compose}
                    onActivate={(event) => {
                      if (event.isTrusted) setCompose(true);
                    }}
                  />
                </div>
              )}
              {compose && binding && (
                <Composer
                  label={text.commentPost}
                  submit={async (body) => {
                    const context = await binding.create(body, null);
                    select(context.thread);
                  }}
                  cancel={() => setCompose(false)}
                  blocked={blocked}
                  archived={archived}
                />
              )}
              {!visibleRows.length && (
                <div className="comments-empty" data-testid="comments-empty">
                  <Check aria-hidden />
                  <h3>{threads.length ? text.commentsEmptyTitle : text.commentsEmptyNone}</h3>
                  <p>
                    {!threads.length
                      ? text.commentsEmptyNoneHint
                      : shownView === 'resolved'
                        ? text.commentsEmptyResolved
                        : text.commentsEmptyCaughtUp}
                  </p>
                </div>
              )}
              <div className="annotation-list">
                {visibleRows.map(({ thread, id, status, shownResolved, needsYou }) => {
                  const isPending = pending[id] !== undefined;
                  const count = thread.comments.filter((value) => !value.deleted).length;
                  const first =
                    thread.comments.find(
                      (value) => !value.deleted && value.ref.writer === thread.ref.writer,
                    ) ?? thread.comments.find((value) => !value.deleted);
                  const rowTitle = threadTitle(thread);
                  const quote = thread.proposal
                    ? `${text.proposalOpen} · ${thread.proposal.proposer.label}`
                    : thread.anchor?.exact;
                  const state = [
                    ...(shownResolved
                      ? [isPending ? text.threadResolved : resolvedLabel(status)]
                      : thread.proposal && thread.decision && !thread.decision.deleted
                        ? [
                            thread.decision.decision === 'approved'
                              ? text.proposalApproved
                              : text.proposalDeclined,
                          ]
                        : []),
                    ...(needsYou ? [text.needsYouWord] : []),
                    ...(shownResolved && status?.unseen ? [text.threadUnseen] : []),
                    ...(thread.anchor && !shownResolved && !resolved.includes(id)
                      ? [text.commentDetached]
                      : []),
                  ];
                  return (
                    <section
                      className="annotation-list-item"
                      data-resolved={shownResolved || undefined}
                      key={id}
                    >
                      <div className="annotation-row-controls">
                        <button
                          className="annotation-row"
                          data-testid="annotation-row"
                          data-thread-id={thread.threadId}
                          data-resolved={shownResolved || undefined}
                          title={Array.from(first?.body.split('\n')[0] ?? '')
                            .slice(0, 32)
                            .join('')}
                          onClick={(event) => {
                            if (event.isTrusted) select(thread.ref);
                          }}
                        >
                          {numbers.get(id) !== undefined && (
                            <span className="annotation-number">{numbers.get(id)}</span>
                          )}
                          <span className="annotation-row-text">
                            <strong>{rowTitle}</strong>
                            {quote && <span className="annotation-row-quote">{quote}</span>}
                            <small
                              data-testid="thread-row-status"
                              data-unseen={status?.unseen || undefined}
                            >
                              {[...(count ? [text.commentCount(count)] : []), ...state].join(' · ')}
                            </small>
                          </span>
                        </button>
                        {!archived &&
                          thread.proposal &&
                          thread.resolved &&
                          status?.controllable &&
                          reopenProposal && (
                            <span className="proposal-row-reopen">
                              <BrowserAction
                                type="button"
                                variant="text"
                                label={text.threadReopen}
                                disabled={blocked}
                                onActivate={(event) => {
                                  if (event.isTrusted) reopenProposal(thread);
                                }}
                              />
                            </span>
                          )}
                      </div>
                    </section>
                  );
                })}
              </div>
            </>
          )
        }
      </BrowserTabs>
    </aside>
  );
}
