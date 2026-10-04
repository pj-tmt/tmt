import { useEffect, useId, useState } from 'react';
import { AskPanel, type AskBinding, type PageAsk } from './ask-panel.js';
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
  const inputId = useId();
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
              cancel?.();
            },
            () => setError(true),
          )
          .finally(() => setBusy(false));
      }}
    >
      {quote && <blockquote>{quote}</blockquote>}
      <label htmlFor={inputId}>{label}</label>
      <textarea
        id={inputId}
        value={body}
        disabled={busy || blocked}
        onChange={(event) => setBody(event.target.value)}
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

function Comment({
  comment,
  thread,
  binding,
  ask,
  asks,
  blocked,
}: {
  comment: CommentView;
  thread: ThreadView;
  binding?: ThreadBinding;
  ask?: AskBinding;
  asks: readonly PageAsk[];
  blocked: boolean;
}) {
  const editId = useId();
  const [now, setNow] = useState(0);
  useEffect(() => setNow(Date.now()), [comment.at]);
  const [editing, setEditing] = useState(false),
    [draft, setDraft] = useState(comment.body),
    [editRevision, setEditRevision] = useState(comment.revision);
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(false);
  const owned = binding?.deviceId === comment.ref.writer;
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
    <article
      className="comment"
      data-testid="comment-entry"
      data-message-id={comment.messageId}
      data-writer={comment.ref.writer}
    >
      <header>
        <p className="comment-byline" title={comment.ref.writer}>
          {owned ? text.askYou : comment.deviceName || text.commentDevice} ·{' '}
          <time
            dateTime={new Date(Number(comment.at)).toISOString()}
            title={new Date(Number(comment.at)).toISOString()}
          >
            {relativeTime(Number(comment.at), now)}
          </time>
          {!comment.deleted && comment.revision !== '1' && <> · {text.commentEdited}</>}
        </p>
      </header>
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
              <label htmlFor={editId}>{text.commentEditBody}</label>
              <textarea
                id={editId}
                value={draft}
                disabled={busy}
                onChange={(event) => setDraft(event.target.value)}
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
          {owned && !editing && (
            <div className="comment-actions">
              <button
                disabled={busy || blocked}
                onClick={(event) => {
                  if (event.isTrusted) {
                    setDraft(comment.body);
                    setEditRevision(comment.revision);
                    setEditing(true);
                  }
                }}
              >
                {text.commentEdit}
              </button>
              <button
                disabled={busy || blocked}
                onClick={(event) => {
                  if (event.isTrusted)
                    action(() => binding!.deleteComment(comment.ref, comment.revision));
                }}
              >
                {text.commentDelete}
              </button>
            </div>
          )}
        </>
      )}
      <AskPanel
        inline
        records={asks.filter(
          (record) =>
            record.thread === thread.threadId && record.messageIds?.includes(comment.messageId),
        )}
        binding={ask}
        blocked={blocked || busy}
      />
      {error && <p role="alert">{text.commentFailed}</p>}
    </article>
  );
}

function Thread({
  thread,
  attached,
  selection,
  binding,
  ask,
  title,
  asks,
  publisher,
  close,
  blocked,
}: {
  thread: ThreadView;
  attached: boolean;
  selection: QuoteSelector | null;
  binding?: ThreadBinding;
  ask?: AskBinding;
  title: string;
  asks: readonly PageAsk[];
  publisher?: string;
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
      <header>
        <strong>
          {thread.deleted
            ? text.threadDeleted
            : thread.resolved
              ? text.threadResolved
              : text.threadOpen}
        </strong>
        {thread.anchor && (
          <span className="comment-status">
            {attached ? text.commentAnchored : text.commentDetached}
          </span>
        )}
      </header>
      {thread.anchor && <blockquote>{thread.anchor.exact}</blockquote>}
      {thread.comments.map((comment) => (
        <Comment
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
              <button
                disabled={blocked || busy}
                onClick={(event) => {
                  if (event.isTrusted) action({ resolved: !thread.resolved });
                }}
              >
                {thread.resolved ? text.threadReopen : text.threadResolve}
              </button>
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
          publisher={publisher}
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
  selection,
  binding,
  ask,
  asks,
  title,
  publisher,
  blocked,
  active,
  select,
  annotation,
  cancelAnnotation,
}: {
  hideHeader?: boolean;
  threads: readonly ThreadView[];
  resolved: readonly string[];
  selection: QuoteSelector | null;
  binding?: ThreadBinding;
  ask?: AskBinding;
  asks: readonly PageAsk[];
  title: string;
  publisher?: string;
  blocked: boolean;
  active: string | null;
  select(ref: DiscussionRef | null): void;
  annotation?: QuoteSelector;
  cancelAnnotation(): void;
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
        {text.commentPage}
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
      {annotation && (
        <section className="annotation-new">
          <blockquote>{annotation.exact}</blockquote>
          <AnnotationInput
            key={JSON.stringify(annotation)}
            binding={ask}
            discussion={binding}
            anchor={annotation}
            asks={asks}
            title={title}
            publisher={publisher}
            blocked={blocked}
            cancel={cancelAnnotation}
            committed={(ref) => {
              cancelAnnotation();
              select(ref);
            }}
          />
        </section>
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
                  selection={selection}
                  binding={binding}
                  ask={ask}
                  asks={asks}
                  title={title}
                  publisher={publisher}
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
