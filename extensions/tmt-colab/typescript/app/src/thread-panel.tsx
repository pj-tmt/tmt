import { Check, CircleCheck, CircleDot, RotateCcw, X } from 'lucide-react';
import { MessageComposer } from './components/message-composer.js';
import { conversationAsks } from './thread-store.js';
import { ConversationTurn } from './components/conversation-turn.js';
import { useEffect, useState, type ReactNode } from 'react';
import { AskPanel, type AskBinding, type PageAsk } from './ask-panel.js';
import { ActionMenu } from './components/action-menu.js';
import type { ThreadBinding } from './thread-store.js';
import type { CommentView, QuoteSelector, ThreadView, DiscussionRef } from './thread-records.js';
import { text } from './strings.js';
import { relativeTime } from './display-time.js';
import { AnnotationInput } from './annotation-input.js';

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
        <button type="submit" disabled={busy || blocked || !body.trim()}>
          {busy ? text.saving : label}
        </button>
        {cancel && (
          <button
            type="button"
            disabled={busy}
            onClick={(event) => {
              if (event.isTrusted) cancel();
            }}
          >
            {text.commentCancel}
          </button>
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
                <button disabled={busy || blocked || !draft.trim()}>{text.commentSave}</button>
                <button
                  type="button"
                  disabled={busy}
                  onClick={(event) => {
                    if (event.isTrusted) setEditing(false);
                  }}
                >
                  {text.commentCancel}
                </button>
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

function Thread({
  thread,
  attached,
  anchorsChecked,
  selection,
  binding,
  ask,
  title,
  asks,
  close,
  blocked,
}: {
  thread: ThreadView;
  attached: boolean;
  anchorsChecked: boolean;
  selection: QuoteSelector | null;
  binding?: ThreadBinding;
  ask?: AskBinding;
  title: string;
  asks: readonly PageAsk[];
  close(): void;
  blocked: boolean;
}) {
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(false);
  const owned = binding?.deviceId === thread.ref.writer;
  const [reattach, setReattach] = useState<QuoteSelector | null>(null);
  const action = (change: Parameters<ThreadBinding['updateThread']>[2]) => {
    if (!binding || busy || blocked) return;
    setBusy(true);
    setError(false);
    void binding
      .updateThread(thread.ref, thread.revision, change)
      .then(
        () => setReattach(null),
        () => setError(true),
      )
      .finally(() => setBusy(false));
  };
  return (
    <section
      className="comment-thread"
      data-testid="comment-thread"
      data-thread-id={thread.threadId}
      data-writer={thread.ref.writer}
      data-anchor={thread.deleted || !thread.anchor ? 'page' : attached ? 'attached' : 'detached'}
    >
      <header className="thread-bar">
        <span className="thread-state">
          {thread.resolved ? <CircleCheck aria-hidden /> : <CircleDot aria-hidden />}
          <strong>
            {thread.deleted
              ? text.threadDeleted
              : thread.resolved
                ? text.threadResolved
                : text.threadOpen}
          </strong>
        </span>
        {thread.anchor && (
          <span className="comment-status">
            {attached ? text.commentAnchored : text.commentDetached}
          </span>
        )}
        <span className="thread-bar-actions">
          {owned && !thread.deleted && (
            <button
              className="thread-action"
              disabled={blocked || busy}
              title={thread.resolved ? text.threadReopen : text.threadResolve}
              aria-label={thread.resolved ? text.threadReopen : text.threadResolve}
              onClick={(event) => {
                if (event.isTrusted) action({ resolved: !thread.resolved });
              }}
            >
              {thread.resolved ? <RotateCcw aria-hidden /> : <Check aria-hidden />}
              {thread.resolved ? text.threadReopen : text.threadResolve}
            </button>
          )}
          <button
            className="thread-action"
            title={text.threadClose}
            aria-label={text.threadClose}
            onClick={(event) => {
              if (event.isTrusted) close();
            }}
          >
            <X aria-hidden />
            {text.threadClose}
          </button>
        </span>
      </header>
      {thread.anchor && <blockquote>{thread.anchor.exact}</blockquote>}
      {thread.anchor && anchorsChecked && !attached && (
        <p className="annotation-hint">{text.commentQuoteChanged}</p>
      )}
      {thread.comments.map((comment) => (
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
      {binding && !thread.deleted && (
        <div className="comment-actions">
          {owned && (
            <>
              {thread.anchor && !attached && (
                <button
                  disabled={blocked || busy || !selection}
                  onClick={(event) => {
                    if (event.isTrusted && selection) setReattach(structuredClone(selection));
                  }}
                >
                  {text.commentReattach}
                </button>
              )}
              <button
                disabled={blocked || busy}
                onClick={(event) => {
                  if (event.isTrusted) action({ deleted: true });
                }}
              >
                {text.threadDelete}
              </button>
            </>
          )}
        </div>
      )}
      {reattach && !thread.deleted && (
        <section className="comment-compose">
          <blockquote>{reattach.exact}</blockquote>
          <div className="comment-actions">
            <button
              disabled={blocked || busy}
              onClick={(event) => {
                if (event.isTrusted) {
                  action({ anchor: reattach });
                }
              }}
            >
              {text.commentConfirmReattach}
            </button>
            <button
              disabled={busy}
              onClick={(event) => {
                if (event.isTrusted) setReattach(null);
              }}
            >
              {text.commentCancel}
            </button>
          </div>
        </section>
      )}
      {!thread.deleted && binding && (
        <AnnotationInput
          binding={ask}
          discussion={binding}
          anchor={thread.anchor}
          thread={thread}
          asks={asks}
          title={title}
          blocked={blocked || busy}
          cancel={close}
          committed={() => {}}
        />
      )}
      {error && <p role="alert">{text.commentFailed}</p>}
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
      <button
        className="comment-page-action"
        disabled={!binding || blocked || compose}
        onClick={(event) => {
          if (event.isTrusted) setCompose(true);
        }}
      >
        + {text.commentPage}
      </button>
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
                <span>
                  {thread.resolved ? text.threadResolved : text.threadOpen}
                  {thread.anchor && !resolved.includes(id) ? ` · ${text.commentDetached}` : ''}
                </span>
              </button>
              {active === id && (
                <Thread
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
                />
              )}
            </section>
          );
        })}
      </div>
    </aside>
  );
}
