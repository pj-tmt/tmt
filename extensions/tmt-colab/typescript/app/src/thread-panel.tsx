import { Check, CircleCheck, CircleDot, RotateCcw, X } from 'lucide-react';
import { BrowserAction, BrowserIconAction } from '@tmt/browser-ui/react';
import { MessageComposer } from './components/message-composer.js';
import type { ComposerEdit } from './components/message-composer-edit.js';
import { conversationAsks } from './thread-store.js';
import { ConversationTurn } from './components/conversation-turn.js';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { AskPanel, type AskBinding, type PageAsk } from './ask-panel.js';
import { ActionMenu } from './components/action-menu.js';
import type { ThreadBinding } from './thread-store.js';
import type { CommentView, QuoteSelector, ThreadView, DiscussionRef } from './thread-records.js';
import { text } from './strings.js';
import { relativeTime } from './display-time.js';
import { AnnotationInput } from './annotation-input.js';
import {
  presentationOf,
  type ThreadPresentation,
  type ThreadStatusOutcome,
} from './thread-status-presentation.js';

/** Only an agent's resolution names its actor; a person's stays the plain state. */
function resolvedLabel(status: ThreadPresentation['status'] | undefined) {
  return status?.actor === 'agent' && status.actorName
    ? text.threadResolvedBy(status.actorName)
    : text.threadResolved;
}

function Composer({
  label,
  quote,
  submit,
  cancel,
  blocked,
}: {
  label: string;
  quote?: string;
  submit(body: string): Promise<void>;
  cancel?(): void;
  blocked: boolean;
}) {
  const [resetKey, setResetKey] = useState(0);
  const [body, setBody] = useState(''),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(false);
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
        onChange={(next) => setBody(next.value)}
      />
      <div className="comment-actions">
        <BrowserAction
          type="submit"
          variant="primary"
          label={label}
          busy={busy}
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
  chat = false,
  status,
  delivery,
}: {
  comment: CommentView;
  chat?: boolean;
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
  const owned = binding?.deviceId === comment.ref.writer;
  const menu = [
    ...(owned && !chat
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
    if (busy || blocked) return;
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
      layout={chat ? 'chat' : 'thread'}
      author={owned ? text.askYou : comment.deviceName || text.commentDevice}
      at={Number(comment.at)}
      authorTitle={comment.ref.writer}
      className="comment"
      data-testid="comment-entry"
      data-message-id={comment.messageId}
      data-writer={comment.ref.writer}
      meta={!comment.deleted && comment.revision !== '1' && <> · {text.commentEdited}</>}
      actions={
        menu.length > 0 &&
        !comment.deleted &&
        !editing && (
          <ActionMenu
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
      delivery={
        <>
          {status}
          {!busy && delivery}
          {error && <p role="alert">{text.commentFailed}</p>}
        </>
      }
    >
      {comment.deleted ? (
        <p className="comment-status">{text.commentDeleted}</p>
      ) : (
        <>
          {editing ? (
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
                autoFocus
                onChange={(next) => setDraft(next.value)}
              />
              <div className="comment-actions">
                <BrowserAction
                  type="submit"
                  variant="primary"
                  label={text.commentSave}
                  busy={busy}
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
            <p className="comment-body">{comment.body}</p>
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
  chat = false,
}: {
  comment: CommentView;
  thread: ThreadView;
  binding?: ThreadBinding;
  ask?: AskBinding;
  asks: readonly PageAsk[];
  blocked: boolean;
  chat?: boolean;
}) {
  const records = conversationAsks(thread, asks, true).filter((record) =>
    record.messageIds?.includes(comment.messageId),
  );
  const user = (status?: ReactNode, delivery?: ReactNode) => (
    <DiscussionComment
      comment={comment}
      binding={binding}
      blocked={blocked}
      chat={chat}
      status={status}
      delivery={delivery}
    />
  );
  return records.length ? (
    <AskPanel
      inline
      chat={chat}
      records={records}
      binding={ask}
      blocked={blocked}
      renderUser={(_record, status, delivery) => user(status, delivery)}
    />
  ) : (
    user()
  );
}

export type ThreadWindowProps = {
  /** Absent until the first committed turn; the composer stays in this same window. */
  thread?: ThreadView;
  anchor?: QuoteSelector;
  layout?: 'panel' | 'anchored';
  attached: boolean;
  anchorsChecked: boolean;
  selection: QuoteSelector | null;
  binding?: ThreadBinding;
  ask?: AskBinding;
  title: string;
  asks: readonly PageAsk[];
  close(): void;
  blocked: boolean;
  observationUnavailable?: boolean;
  composer?: ReactNode;
  initialEdit?: ComposerEdit;
  onDraft?(edit: ComposerEdit): void;
  onBusy?(busy: boolean): void;
  /** Presentation inputs for this thread, computed once by the authenticated parent. */
  status?: ThreadPresentation['status'];
  /** The parent owns recipient freezing and notification; the window only asks. */
  onStatusChange?(resolved: boolean): Promise<ThreadStatusOutcome>;
};

export function ThreadWindow({
  thread,
  anchor: initialAnchor,
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
  observationUnavailable,
  composer,
  initialEdit,
  onDraft,
  onBusy,
  status,
  onStatusChange,
}: ThreadWindowProps) {
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(false),
    [notNotified, setNotNotified] = useState<string[]>([]);
  const owned = !!thread && binding?.deviceId === thread.ref.writer;
  const anchor = thread ? thread.anchor : initialAnchor;
  // A resolved thread leaves the page's marker set, so its attachment is not
  // observed until Reopen restores the marker.
  const tracked = !!anchor && !thread?.resolved;
  const history = useRef<HTMLDivElement>(null);
  const previousComments = useRef<Set<string>>(undefined);
  const previousReplies = useRef(new Map<string, string>());
  const arrival = useRef<{ offset: number; scroll(): void }>(undefined);
  useEffect(() => {
    const node = history.current;
    if (layout !== 'anchored' || !node) return;
    const observer = new ResizeObserver(() => {
      const target = arrival.current;
      if (
        target &&
        (Math.abs(node.scrollTop - target.offset) < 1 ||
          Math.abs(node.scrollTop - Math.max(0, node.scrollHeight - node.clientHeight)) < 1)
      )
        target.scroll();
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, [layout]);
  useEffect(() => {
    const node = history.current;
    if (layout !== 'anchored' || !node) return;
    const comments = new Set(
      thread?.comments.map((comment) => `${comment.ref.writer}:${comment.messageId}`),
    );
    const opened = previousComments.current === undefined;
    const newComment = [...comments].some((id) => !previousComments.current?.has(id));
    previousComments.current = comments;
    const records = conversationAsks(thread, asks, true);
    const changed = records.filter(
      (record) =>
        record.reply !== undefined &&
        previousReplies.current.get(record.operationId) !== record.reply,
    );
    previousReplies.current = new Map(
      records
        .filter((record) => record.reply !== undefined)
        .map((record) => [record.operationId, record.reply!]),
    );
    if (!opened && !newComment && !changed.length) return;
    // A delayed answer may belong to an earlier turn. Scroll this message area,
    // never the document, so that newly arrived answer is readable too.
    const latest = changed.at(-1);
    const reply =
      latest &&
      Array.from(node.querySelectorAll<HTMLElement>('[data-operation-id]'))
        .find((entry) => entry.dataset.operationId === latest.operationId)
        ?.querySelector<HTMLElement>('[data-testid="ask-reply"]');
    const scroll = () => {
      node.scrollTop = node.scrollHeight;
      if (reply?.isConnected)
        node.scrollTop +=
          reply.getBoundingClientRect().bottom - node.getBoundingClientRect().bottom;
      arrival.current = { offset: node.scrollTop, scroll };
    };
    scroll();
  }, [thread, asks, layout]);
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
    let closeAfter = false;
    void Promise.resolve()
      .then(() => onStatusChange(resolved))
      .then(
        (result) => {
          const names = (result.warnings ?? []).map((warning) => warning.name);
          setReattach(null);
          setNotNotified(names);
          // Keep the window open while it still has something to tell the person.
          closeAfter = result.changed && resolved && layout === 'anchored' && !names.length;
        },
        () => setError(true),
      )
      .finally(() => {
        setBusy(false);
        onBusy?.(false);
        if (closeAfter) close();
      });
  };
  return (
    <section
      className="comment-thread"
      data-layout={layout}
      data-testid={thread ? 'comment-thread' : 'annotation-window'}
      data-thread-id={thread?.threadId}
      data-writer={thread?.ref.writer}
      data-anchor={
        thread?.deleted || !anchor
          ? 'page'
          : thread?.resolved
            ? 'resolved'
            : attached
              ? 'attached'
              : 'detached'
      }
    >
      <header className="thread-bar">
        <span className="thread-state">
          {thread?.resolved ? <CircleCheck aria-hidden /> : <CircleDot aria-hidden />}
          <strong>
            {!thread
              ? text.annotationTitle
              : thread.deleted
                ? text.threadDeleted
                : thread.resolved
                  ? resolvedLabel(status)
                  : text.threadOpen}
          </strong>
        </span>
        {tracked && thread && (
          <span className="comment-status">
            {attached ? text.commentAnchored : text.commentDetached}
          </span>
        )}
        <span className="thread-bar-actions">
          {thread && onStatusChange && status?.controllable && !thread.deleted && (
            <BrowserIconAction
              type="button"
              variant="text"
              disabled={blocked}
              busy={busy}
              label={thread.resolved ? text.threadReopen : text.threadResolve}
              icon={thread.resolved ? <RotateCcw /> : <Check />}
              onActivate={(event) => {
                if (event.isTrusted) changeStatus(!thread.resolved);
              }}
            />
          )}
          <BrowserIconAction
            type="button"
            variant="text"
            label={thread ? text.threadClose : text.annotationClose}
            icon={<X />}
            busy={busy}
            onActivate={(event) => {
              if (event.isTrusted) close();
            }}
          />
        </span>
      </header>
      {observationUnavailable && (
        <p role="status" className="annotation-hint">
          {text.askObservationUnavailable}
        </p>
      )}
      <div className="thread-messages" ref={history}>
        {anchor && <blockquote>{anchor.exact}</blockquote>}
        {tracked && anchorsChecked && !attached && (
          <p className="annotation-hint">{text.commentQuoteChanged}</p>
        )}
        {thread?.comments.map((comment) => (
          <CommentExchange
            key={`${comment.ref.writer}:${comment.messageId}`}
            comment={comment}
            thread={thread}
            binding={binding}
            ask={ask}
            asks={asks}
            blocked={blocked || busy}
          />
        ))}
        {binding && thread && !thread.deleted && owned && (
          <div className="comment-actions">
            {tracked && !attached && (
              <BrowserAction
                type="button"
                variant="text"
                label={text.commentReattach}
                disabled={blocked || busy || !selection}
                onActivate={(event) => {
                  if (event.isTrusted && selection) setReattach(structuredClone(selection));
                }}
              />
            )}
            <BrowserAction
              type="button"
              variant="text"
              label={text.threadDelete}
              disabled={blocked}
              busy={busy}
              onActivate={(event) => {
                if (event.isTrusted) action({ deleted: true });
              }}
            />
          </div>
        )}
        {reattach && thread && !thread.deleted && (
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
        {notNotified.length > 0 && (
          <p role="status" className="annotation-hint">
            {text.threadNotNotified(notNotified.join(', '))}
          </p>
        )}
        {error && <p role="alert">{text.commentFailed}</p>}
      </div>
      {composer !== undefined
        ? composer
        : thread &&
          !thread.deleted &&
          binding && (
            <AnnotationInput
              binding={ask}
              discussion={binding}
              anchor={thread.anchor}
              thread={thread}
              asks={asks}
              title={title}
              blocked={blocked || busy}
              initialEdit={initialEdit}
              onDraft={(_value, edit) => onDraft?.(edit)}
              cancel={close}
              committed={() => {}}
            />
          )}
    </section>
  );
}

export function ThreadPanel({
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
  active,
  select,
  draft,
  onDraft,
  presentations,
  onStatusChange,
  onBusy,
}: {
  hideHeader?: boolean;
  threads: readonly ThreadView[];
  resolved: readonly string[];
  anchorsChecked: boolean;
  selection: QuoteSelector | null;
  binding?: ThreadBinding;
  ask?: AskBinding;
  asks: readonly PageAsk[];
  title: string;
  blocked: boolean;
  active: string | null;
  select(ref: DiscussionRef | null): void;
  draft?(ref: DiscussionRef): ComposerEdit | undefined;
  onDraft?(ref: DiscussionRef, edit: ComposerEdit): void;
  presentations?: readonly ThreadPresentation[];
  onStatusChange?(thread: ThreadView, resolved: boolean): Promise<ThreadStatusOutcome>;
  onBusy?(busy: boolean): void;
}) {
  const [compose, setCompose] = useState(false);
  const [now, setNow] = useState(0);
  useEffect(() => setNow(Date.now()), [threads, asks]);
  return (
    <aside className="comments-panel" aria-label={text.comments} data-testid="comments-panel">
      {!hideHeader && (
        <header>
          <h2>{text.comments}</h2>
          <span>{threads.filter((value) => !value.deleted).length}</span>
        </header>
      )}
      <BrowserAction
        type="button"
        variant="text"
        label={`+ ${text.commentPage}`}
        disabled={!binding || blocked || compose}
        onActivate={(event) => {
          if (event.isTrusted) setCompose(true);
        }}
      />
      {compose && binding && (
        <Composer
          label={text.commentPost}
          submit={async (body) => {
            const context = await binding.create(body, null);
            select(context.thread);
          }}
          cancel={() => setCompose(false)}
          blocked={blocked}
        />
      )}
      {!threads.length && <p className="comment-status">{text.commentEmpty}</p>}
      <div className="annotation-list">
        {threads.map((thread) => {
          const id = `${thread.ref.writer}:${thread.threadId}`;
          const participants = [
            ...new Set([
              ...thread.comments
                .filter((value) => !value.deleted)
                .map((value) => value.deviceName || text.commentDevice),
              ...asks
                .filter((value) => value.thread === thread.threadId)
                .map((value) => value.agentName),
            ]),
          ];
          const at = Number(thread.comments.at(-1)?.at ?? thread.at);
          const status = presentationOf(presentations, thread.ref)?.status;
          return (
            <section className="annotation-list-item" key={id}>
              <button
                className="annotation-row"
                data-testid="annotation-row"
                data-thread-id={thread.threadId}
                title={Array.from(
                  (
                    thread.comments.find(
                      (value) => !value.deleted && value.ref.writer === thread.ref.writer,
                    ) ?? thread.comments.find((value) => !value.deleted)
                  )?.body.split('\n')[0] ?? '',
                )
                  .slice(0, 32)
                  .join('')}
                aria-expanded={active === id}
                onClick={(event) => {
                  if (event.isTrusted) select(thread.ref);
                }}
              >
                <strong>
                  {thread.deleted ? text.threadDeleted : (thread.anchor?.exact ?? 'Page comments')}
                </strong>
                <span>
                  {participants.join(', ')} ·{' '}
                  <time dateTime={new Date(at).toISOString()}>{relativeTime(at, now)}</time>
                </span>
                <span data-testid="thread-row-status" data-unseen={status?.unseen || undefined}>
                  {thread.resolved ? <CircleCheck aria-hidden /> : <CircleDot aria-hidden />}
                  {thread.resolved ? resolvedLabel(status) : text.threadOpen}
                  {status?.unseen ? ` · ${text.threadUnseen}` : ''}
                  {thread.anchor && !thread.resolved && !resolved.includes(id)
                    ? ` · ${text.commentDetached}`
                    : ''}
                </span>
              </button>
              {active === id && (
                <ThreadWindow
                  thread={thread}
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
                  onStatusChange={
                    onStatusChange && ((resolved) => onStatusChange(thread, resolved))
                  }
                />
              )}
            </section>
          );
        })}
      </div>
    </aside>
  );
}
