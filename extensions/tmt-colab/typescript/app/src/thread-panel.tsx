import { useEffect, useId, useState } from 'react';
import { AskControl, type AskBinding } from './ask-panel.js';
import type { ThreadBinding } from './thread-store.js';
import type { CommentView, QuoteSelector, ThreadView } from './thread-records.js';
import { text } from './strings.js';
import { relativeTime } from './display-time.js';

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
  title,
  blocked,
}: {
  comment: CommentView;
  thread: ThreadView;
  binding?: ThreadBinding;
  ask?: AskBinding;
  title: string;
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
      <AskControl
        originUnavailable={thread.deleted || comment.deleted}
        binding={ask}
        selection={thread.anchor?.exact ?? ''}
        title={title}
        blocked={blocked || busy}
        origin={{
          body: comment.body,
          context: {
            thread: thread.ref,
            message: comment.ref,
            threadRevision: thread.revision,
            messageRevision: comment.revision,
          },
        }}
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
  blocked,
}: {
  thread: ThreadView;
  attached: boolean;
  selection: QuoteSelector | null;
  binding?: ThreadBinding;
  ask?: AskBinding;
  title: string;
  blocked: boolean;
}) {
  const [replying, setReplying] = useState(false),
    [busy, setBusy] = useState(false),
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
          title={title}
          blocked={blocked || busy}
        />
      ))}
      {binding && !thread.deleted && (
        <div className="comment-actions">
          <button
            disabled={blocked || busy || replying}
            onClick={(event) => {
              if (event.isTrusted) setReplying(true);
            }}
          >
            {text.commentReply}
          </button>
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
      {replying && !thread.deleted && binding && (
        <Composer
          label={text.commentPostReply}
          submit={(body) => binding.reply(thread.ref, body)}
          cancel={() => setReplying(false)}
          blocked={blocked || busy}
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
  title,
  blocked,
}: {
  hideHeader?: boolean;
  threads: readonly ThreadView[];
  resolved: readonly string[];
  selection: QuoteSelector | null;
  binding?: ThreadBinding;
  ask?: AskBinding;
  title: string;
  blocked: boolean;
}) {
  const [compose, setCompose] = useState<{ anchor: QuoteSelector | null } | null>(null);
  return (
    <aside className="comments-panel" aria-label={text.comments} data-testid="comments-panel">
      {!hideHeader && (
        <header>
          <h2>{text.comments}</h2>
          <span>{threads.filter((v) => !v.deleted).length}</span>
        </header>
      )}
      <div className="comment-actions">
        <button
          data-testid="comment-action"
          disabled={!binding || blocked || !!compose || !selection}
          onClick={(event) => {
            if (event.isTrusted && selection) setCompose({ anchor: structuredClone(selection) });
          }}
        >
          {text.commentSelection}
        </button>
        <button
          disabled={!binding || blocked || !!compose}
          onClick={(event) => {
            if (event.isTrusted) setCompose({ anchor: null });
          }}
        >
          {text.commentPage}
        </button>
      </div>
      {compose && binding && (
        <Composer
          label={text.commentPost}
          quote={compose.anchor?.exact}
          submit={(body) => binding.create(body, compose.anchor)}
          cancel={() => setCompose(null)}
          blocked={blocked}
        />
      )}
      {!threads.length && <p className="comment-status">{text.commentEmpty}</p>}
      {threads.map((thread) => (
        <Thread
          key={`${thread.ref.writer}:${thread.threadId}`}
          thread={thread}
          attached={resolved.includes(`${thread.ref.writer}:${thread.threadId}`)}
          selection={selection}
          binding={binding}
          ask={ask}
          title={title}
          blocked={blocked}
        />
      ))}
    </aside>
  );
}
