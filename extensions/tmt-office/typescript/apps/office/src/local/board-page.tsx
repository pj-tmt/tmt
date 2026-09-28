import { useCallback, useContext, useEffect, useRef, useState } from 'react';
import { LocalRuntimeContext } from './local-runtime.js';
import type { LocalRuntime } from './local-runtime.js';
import type {
  BoardCategory,
  BoardEntry,
  BoardShowPage,
  BoardThreadSummary,
} from './board-contract.js';
import './board.css';
import { BoardReferenceActions, BoardShare } from './board-share.js';
import { boardCategoryKey as categoryKey } from './board-contract.js';
import { BoardNavigation, useBoardNavigation } from './board-navigation.js';
import { boardErrorMessage as message, useBoardMutation } from './use-board-mutation.js';

function categoryLabel(category: BoardCategory, rooms: Map<string, string> = new Map()): string {
  switch (category.kind) {
    case 'general':
      return 'General';
    case 'repository':
      return category.repositoryId;
    case 'room':
      return `Room · ${rooms.get(category.roomId) ?? category.roomId}`;
  }
}

function authorLabel(entry: Pick<BoardEntry, 'author'>): string {
  return entry.author.kind === 'owner' ? 'Office owner' : entry.author.name;
}

function timeLabel(milliseconds: number): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(milliseconds));
}

export function LocalBoardPage({ entryCategory }: { entryCategory?: BoardCategory } = {}) {
  return (
    <BoardNavigation>
      <BoardWorkspace entryCategory={entryCategory} />
    </BoardNavigation>
  );
}

function BoardWorkspace({ entryCategory }: { entryCategory?: BoardCategory }) {
  const { request: requestNavigation, epoch } = useBoardNavigation();
  const runtime = useContext(LocalRuntimeContext);
  const [categories, setCategories] = useState<BoardCategory[]>([]);
  const [categoryCursor, setCategoryCursor] = useState<string | null>(null);
  const [category, setCategory] = useState<BoardCategory>(entryCategory ?? { kind: 'general' });
  const [roomNames, setRoomNames] = useState(new Map<string, string>());
  const [view, setView] = useState<'recent' | 'updated'>('updated');
  const [pane, setPane] = useState<'threads' | 'discussion'>('threads');
  const [composing, setComposing] = useState(false);
  const [sharedThread, setSharedThread] = useState<BoardEntry>();
  const shareTrigger = useRef<HTMLButtonElement | null>(null);
  const threadTrigger = useRef<HTMLButtonElement | null>(null);
  const [threads, setThreads] = useState<BoardThreadSummary[]>([]);
  const [threadCursor, setThreadCursor] = useState<string | null>(null);
  const [selected, setSelected] = useState<string>();
  const [detail, setDetail] = useState<BoardShowPage>();
  const [detailVersion, setDetailVersion] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const refreshGeneration = useRef(0);
  const categoryContinuation = useRef<string | undefined>(undefined);
  const threadContinuation = useRef<string | undefined>(undefined);
  const selectedCategoryKey = categoryKey(category);
  const selectCategory = useCallback(
    (next: BoardCategory) => {
      if (categoryKey(next) === selectedCategoryKey) return;
      requestNavigation(() => {
        ++refreshGeneration.current;
        threadTrigger.current = null;
        setCategory(next);
        setSelected(undefined);
        setDetail(undefined);
        setThreads([]);
        setSharedThread(undefined);
        setComposing(false);
        setPane('threads');
      });
    },
    [requestNavigation, selectedCategoryKey]
  );
  const lastEntry = useRef(entryCategory);
  useEffect(() => {
    if (lastEntry.current === entryCategory) return;
    lastEntry.current = entryCategory;
    if (entryCategory) selectCategory(entryCategory);
  }, [entryCategory, selectCategory]);

  useEffect(() => {
    if (pane === 'threads') threadTrigger.current?.focus();
  }, [pane]);
  useEffect(() => {
    if (!sharedThread) shareTrigger.current?.focus();
  }, [sharedThread]);

  async function loadMoreCategories(activeRuntime: LocalRuntime, cursor: string) {
    if (categoryContinuation.current === cursor) return;
    categoryContinuation.current = cursor;
    const generation = refreshGeneration.current;
    try {
      const page = await activeRuntime.board.categories({ limit: 20, cursor });
      if (generation !== refreshGeneration.current) return;
      setCategories((current) => {
        const seen = new Set<string>();
        return [...current, ...page.categories].filter((item) => {
          const key = categoryKey(item);
          if (seen.has(key)) return false;
          seen.add(key);
          return true;
        });
      });
      setCategoryCursor(page.nextCursor);
    } catch (caught) {
      if (generation !== refreshGeneration.current) return;
      setCategoryCursor(null);
      setError(message(caught));
    } finally {
      if (categoryContinuation.current === cursor) categoryContinuation.current = undefined;
    }
  }

  async function loadMoreThreads(activeRuntime: LocalRuntime, cursor: string) {
    if (threadContinuation.current === cursor) return;
    threadContinuation.current = cursor;
    const generation = refreshGeneration.current;
    try {
      const page = await activeRuntime.board.list({ category, view, limit: 20, cursor });
      if (generation !== refreshGeneration.current) return;
      setThreads((current) => [...current, ...page.threads]);
      setThreadCursor(page.nextCursor);
    } catch (caught) {
      if (generation !== refreshGeneration.current) return;
      setThreadCursor(null);
      setError(message(caught));
    } finally {
      if (threadContinuation.current === cursor) threadContinuation.current = undefined;
    }
  }

  async function refresh(activeRuntime = runtime) {
    if (!activeRuntime) return;
    const generation = ++refreshGeneration.current;
    const requestedCategory = category;
    const requestedView = view;
    setError(undefined);
    setLoading(true);
    try {
      const [categoryPage, threadPage, rooms] = await Promise.all([
        activeRuntime.board.categories({ limit: 20 }),
        activeRuntime.board.list({ category: requestedCategory, view: requestedView, limit: 20 }),
        activeRuntime.rooms.list().catch(() => []),
      ]);
      if (generation !== refreshGeneration.current) return;
      setRoomNames(new Map(rooms.map((room) => [room.id, room.name])));
      setCategories((_current) => {
        const merged = [...categoryPage.categories];
        if (!merged.some((item) => categoryKey(item) === categoryKey(requestedCategory))) {
          merged.push(requestedCategory);
        }
        return merged;
      });
      setCategoryCursor(categoryPage.nextCursor);
      setThreads(threadPage.threads);
      setThreadCursor(threadPage.nextCursor);
      // Refresh/order/post completion must not dispose another thread's form.
      // Explicit category navigation clears selection before loading its first page.
      setSelected((current) => current ?? threadPage.threads[0]?.id);
      setDetailVersion((version) => version + 1);
    } catch (caught) {
      if (generation === refreshGeneration.current) setError(message(caught));
    } finally {
      if (generation === refreshGeneration.current) setLoading(false);
    }
  }

  useEffect(() => {
    void refresh(runtime);
    // The runtime is installation-scoped and the serialized category is an explicit reload key.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime, selectedCategoryKey, view]);

  useEffect(() => {
    let active = true;
    if (!runtime || !selected) return;
    void runtime.board
      .show({ threadId: selected, replyLimit: 20 })
      .then((page) => {
        if (active) setDetail(page);
      })
      .catch((caught) => {
        if (active) setError(message(caught));
      });
    return () => {
      active = false;
    };
  }, [runtime, selected, detailVersion]);

  const visibleDetail = detail?.thread.id === selected ? detail : undefined;

  if (!runtime) return <p role="alert">This build does not provide the local Office runtime.</p>;
  return (
    <section className="board-page" data-pane={pane}>
      {sharedThread && (
        <BoardShare
          key={`${sharedThread.id}:${epoch}`}
          thread={sharedThread}
          runtime={runtime}
          close={() => setSharedThread(undefined)}
        />
      )}
      <div className="board-content" hidden={Boolean(sharedThread)}>
        <div className="board-heading">
          <div>
            <p className="eyebrow">Installation-local discussion</p>
            <h1>Discussion board</h1>
            <p className="intro">Ideas, discoveries, and work in progress.</p>
          </div>
          <div className="board-heading-actions">
            <button type="button" className="board-secondary" onClick={() => void refresh()}>
              Refresh board
            </button>
            <button
              type="button"
              className="board-primary"
              aria-expanded={composing}
              onClick={() => setComposing(true)}
            >
              New post
            </button>
          </div>
        </div>

        <div className="board-filters" aria-label="Board filters">
          <div className="board-categories" role="group" aria-label="Category">
            {categories.map((item) => (
              <button
                type="button"
                key={categoryKey(item)}
                aria-pressed={categoryKey(item) === selectedCategoryKey}
                title={categoryLabel(item, roomNames)}
                onClick={() => selectCategory(item)}
              >
                {categoryLabel(item, roomNames)}
              </button>
            ))}
          </div>
          <label>
            Order
            <select
              value={view}
              onChange={(event) => setView(event.target.value as 'recent' | 'updated')}
            >
              <option value="recent">Newest threads</option>
              <option value="updated">Recently active</option>
            </select>
          </label>
          {categoryCursor && (
            <button
              type="button"
              className="board-secondary"
              onClick={() => void loadMoreCategories(runtime, categoryCursor)}
            >
              More categories
            </button>
          )}
        </div>

        {error && (
          <p role="alert" className="board-alert">
            {error}
          </p>
        )}
        {loading && <p role="status">Loading the board…</p>}
        <NewThreadForm
          key={`${selectedCategoryKey}:${epoch}`}
          runtime={runtime}
          category={category}
          categoryName={categoryLabel(category, roomNames)}
          open={composing}
          setOpen={setComposing}
          completed={() => {
            void refresh(runtime);
            setPane('discussion');
          }}
        />
        <div className="board-layout" aria-busy={loading}>
          <aside className="board-index" aria-label="Discussion threads">
            {threads.length === 0 ? (
              <p className="board-empty">No posts in this category yet.</p>
            ) : (
              <ol className="board-thread-list">
                {threads.map((thread) => (
                  <li key={thread.id}>
                    <button
                      type="button"
                      aria-current={selected === thread.id ? 'true' : undefined}
                      onClick={(event) => {
                        const trigger = event.currentTarget;
                        const select = () => {
                          threadTrigger.current = trigger;
                          setSelected(thread.id);
                          setPane('discussion');
                        };
                        if (selected === thread.id) select();
                        else requestNavigation(select);
                      }}
                    >
                      <strong>{thread.deleted ? 'Deleted post' : thread.title}</strong>
                      <span>
                        {authorLabel(thread)} · {thread.replyCount} replies
                      </span>
                      <span>{timeLabel(thread.updatedAtMs)}</span>
                    </button>
                  </li>
                ))}
              </ol>
            )}
            {threadCursor && (
              <button
                type="button"
                className="board-secondary board-more"
                onClick={() => void loadMoreThreads(runtime, threadCursor)}
              >
                More threads
              </button>
            )}
          </aside>
          <section
            className="board-conversation"
            id="board-conversation"
            aria-label="Selected discussion"
          >
            <button
              type="button"
              className="board-secondary board-back"
              onClick={() => setPane('threads')}
            >
              ← Back to threads
            </button>
            {selected && !visibleDetail ? (
              <p role="status">Loading this discussion…</p>
            ) : visibleDetail ? (
              <ThreadView
                key={`${visibleDetail.thread.id}:${epoch}`}
                runtime={runtime}
                page={visibleDetail}
                ask={(trigger) => {
                  shareTrigger.current = trigger;
                  setSharedThread({ ...visibleDetail.thread });
                }}
                changed={() => {
                  void refresh(runtime);
                }}
                setError={setError}
              />
            ) : (
              <div className="board-empty board-empty-detail">
                <h2>Start the conversation</h2>
                <p>Create the first post for {categoryLabel(category, roomNames)}.</p>
              </div>
            )}
          </section>
        </div>
      </div>
    </section>
  );
}

function NewThreadForm({
  runtime,
  category,
  categoryName,
  open,
  setOpen,
  completed,
}: {
  runtime: LocalRuntime;
  category: BoardCategory;
  categoryName: string;
  open: boolean;
  setOpen(open: boolean): void;
  completed(id: string): void;
}) {
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const mutation = useBoardMutation<{ category: BoardCategory; title: string; body: string }>(
    Boolean(title || body)
  );
  if (!open) return null;
  return (
    <form
      className="board-compose"
      onSubmit={(event) => {
        event.preventDefault();
        void mutation.run(
          { category, title, body },
          (input) => runtime.board.post(input),
          (receipt) => {
            setTitle('');
            setBody('');
            setOpen(false);
            completed(receipt.threadId);
          }
        );
      }}
    >
      <h2>New post</h2>
      <p className="board-empty">
        Posting to {categoryName}. Sharing here does not dispatch a task.
      </p>
      <label>
        Title
        <input
          required
          disabled={mutation.locked}
          value={title}
          maxLength={160}
          onChange={(event) => {
            setTitle(event.target.value);
          }}
        />
      </label>
      <label>
        Message
        <textarea
          required
          disabled={mutation.locked}
          value={body}
          rows={6}
          onChange={(event) => {
            setBody(event.target.value);
          }}
        />
      </label>
      {mutation.error && <p role="alert">{mutation.error}</p>}
      {mutation.uncertain && (
        <p>
          An attempt may already be saved. Retry uses the same operation; discarding only forgets
          this retry.
        </p>
      )}
      <div className="board-actions">
        <button type="submit" className="board-primary" disabled={mutation.busy}>
          {mutation.busy ? 'Posting…' : 'Post as owner'}
        </button>
        <button
          type="button"
          className="board-secondary"
          disabled={mutation.busy}
          onClick={() => setOpen(false)}
        >
          Close draft
        </button>
        <button
          type="button"
          className="board-danger"
          disabled={mutation.busy}
          onClick={() => {
            mutation.discard();
            setTitle('');
            setBody('');
            setOpen(false);
          }}
        >
          Discard draft
        </button>
      </div>
    </form>
  );
}

function ThreadView({
  runtime,
  page,
  ask,
  changed,
  setError,
}: {
  runtime: LocalRuntime;
  page: BoardShowPage;
  ask(trigger: HTMLButtonElement): void;
  changed(): void;
  setError(value?: string): void;
}) {
  const [replies, setReplies] = useState(page.replies);
  const [nextCursor, setNextCursor] = useState(page.nextCursor);
  const replyContinuation = useRef<string | undefined>(undefined);
  const currentPage = useRef(page);
  if (currentPage.current !== page) {
    currentPage.current = page;
    replyContinuation.current = undefined;
  }
  useEffect(() => {
    setReplies(page.replies);
    setNextCursor(page.nextCursor);
  }, [page]);
  return (
    <article className="board-thread">
      <EntryCard runtime={runtime} entry={page.thread} changed={changed} />
      <BoardReferenceActions key={page.thread.id} thread={page.thread} ask={ask} />
      <section className="board-replies" aria-labelledby="replies-title">
        <h2 id="replies-title">Replies</h2>
        {replies.length === 0 ? (
          <p className="board-empty">No replies yet.</p>
        ) : (
          replies.map((reply) => (
            <EntryCard key={reply.id} runtime={runtime} entry={reply} changed={changed} />
          ))
        )}
        {nextCursor && (
          <button
            type="button"
            className="board-secondary"
            onClick={() => {
              const cursor = nextCursor;
              if (!cursor || replyContinuation.current === cursor) return;
              replyContinuation.current = cursor;
              const requestedPage = page;
              void runtime.board
                .show({ threadId: page.thread.id, replyLimit: 20, replyCursor: cursor })
                .then((next) => {
                  if (currentPage.current !== requestedPage) return;
                  setReplies((current) => [...current, ...next.replies]);
                  setNextCursor(next.nextCursor);
                })
                .catch((caught) => {
                  if (currentPage.current !== requestedPage) return;
                  setNextCursor(null);
                  setError(message(caught));
                })
                .finally(() => {
                  if (replyContinuation.current === cursor) replyContinuation.current = undefined;
                });
            }}
          >
            More replies
          </button>
        )}
      </section>
      <ReplyForm
        runtime={runtime}
        threadId={page.thread.id}
        closed={page.thread.deleted}
        completed={changed}
      />
    </article>
  );
}

function EntryCard({
  runtime,
  entry,
  changed,
}: {
  runtime: LocalRuntime;
  entry: BoardEntry;
  changed(): void;
}) {
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(entry.title ?? '');
  const [body, setBody] = useState(entry.body ?? '');
  const edit = useBoardMutation<{
    entryId: string;
    title?: string;
    body: string;
    ifRevision: number;
  }>(editing);
  const removal = useBoardMutation<{
    entryId: string;
    ifRevision: number;
    moderate: boolean;
  }>(false);
  const owner = entry.author.kind === 'owner';
  const root = entry.id === entry.threadId;
  useEffect(() => {
    if (!editing) {
      setTitle(entry.title ?? '');
      setBody(entry.body ?? '');
    }
  }, [editing, entry.body, entry.revision, entry.title]);
  return (
    <article className={`board-entry${entry.deleted ? ' board-tombstone' : ''}`}>
      {editing ? (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void edit.run(
              {
                entryId: entry.id,
                ...(root ? { title } : {}),
                body,
                ifRevision: entry.revision,
              },
              (input) => runtime.board.edit(input),
              () => {
                setEditing(false);
                changed();
              }
            );
          }}
        >
          {root && (
            <label>
              Title
              <input
                required
                disabled={edit.locked}
                value={title}
                maxLength={160}
                onChange={(event) => {
                  setTitle(event.target.value);
                }}
              />
            </label>
          )}
          <label>
            Message
            <textarea
              required
              disabled={edit.locked}
              rows={6}
              value={body}
              onChange={(event) => {
                setBody(event.target.value);
              }}
            />
          </label>
          {edit.error && <p role="alert">{edit.error}</p>}
          {edit.uncertain && (
            <p>
              An edit may already be saved. Retry keeps the original revision and operation;
              discarding does not undo it.
            </p>
          )}
          <div className="board-actions">
            <button className="board-primary" disabled={edit.busy} type="submit">
              Save edit
            </button>
            <button
              className="board-secondary"
              disabled={edit.busy}
              type="button"
              onClick={() => {
                edit.discard();
                setEditing(false);
              }}
            >
              Discard edit
            </button>
          </div>
        </form>
      ) : (
        <>
          {root && <h2>{entry.deleted ? 'Deleted post' : entry.title}</h2>}
          <p className="board-body">{entry.deleted ? 'Deleted entry' : entry.body}</p>
          <EntryMeta entry={entry} />
          <div className="board-actions">
            {owner && !entry.deleted && (
              <button
                type="button"
                className="board-secondary"
                disabled={removal.locked}
                onClick={() => setEditing(true)}
              >
                Edit
              </button>
            )}
            {(!entry.deleted || removal.locked) && (
              <button
                type="button"
                className="board-danger"
                disabled={removal.busy}
                onClick={() => {
                  const moderate = !owner;
                  if (
                    moderate &&
                    !window.confirm(
                      'Delete this entry as the Office owner? Its content will be cleared; attribution remains.'
                    )
                  )
                    return;
                  void removal.run(
                    {
                      entryId: entry.id,
                      ifRevision: entry.revision,
                      moderate,
                    },
                    (input) => runtime.board.delete(input),
                    changed
                  );
                }}
              >
                {owner ? 'Delete' : 'Moderate delete'}
              </button>
            )}
          </div>
          {removal.error && <p role="alert">{removal.error}</p>}
          {removal.uncertain && (
            <>
              <p>Deletion may already have happened. Retry uses the same operation.</p>
              <button onClick={() => removal.discard()}>Stop retrying deletion</button>
            </>
          )}
        </>
      )}
    </article>
  );
}

function EntryMeta({ entry }: { entry: BoardEntry }) {
  return (
    <p className="board-meta">
      {authorLabel(entry)} · revision {entry.revision} · {timeLabel(entry.updatedAtMs)}
    </p>
  );
}

function ReplyForm({
  runtime,
  threadId,
  closed,
  completed,
}: {
  runtime: LocalRuntime;
  threadId: string;
  closed: boolean;
  completed(): void;
}) {
  const [body, setBody] = useState('');
  const mutation = useBoardMutation<{ threadId: string; body: string }>(Boolean(body));
  if (closed && !body && !mutation.locked) return null;
  return (
    <form
      className="board-compose board-reply-form"
      onSubmit={(event) => {
        event.preventDefault();
        if (closed && !mutation.locked) return;
        void mutation.run(
          { threadId, body },
          (input) => runtime.board.reply(input),
          () => {
            setBody('');
            completed();
          }
        );
      }}
    >
      <h2>Reply</h2>
      {closed && (
        <p>
          This thread was deleted. Your draft is retained; only an existing operation can be
          retried.
        </p>
      )}
      <label>
        Message
        <textarea
          required
          disabled={mutation.locked}
          rows={5}
          value={body}
          onChange={(event) => {
            setBody(event.target.value);
          }}
        />
      </label>
      {mutation.error && <p role="alert">{mutation.error}</p>}
      {mutation.uncertain && (
        <p>
          A reply may already be saved. Retry keeps the original thread and operation; discard only
          forgets the retry.
        </p>
      )}
      <button
        type="submit"
        className="board-primary"
        disabled={mutation.busy || (closed && !mutation.locked)}
      >
        {mutation.busy ? 'Replying…' : 'Reply as owner'}
      </button>
      <button
        type="button"
        disabled={mutation.busy}
        onClick={() => {
          mutation.discard();
          setBody('');
        }}
      >
        Discard reply
      </button>
    </form>
  );
}
