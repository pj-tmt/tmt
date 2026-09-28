import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import type { BoardEntry } from './board-contract.js';
import type { LocalRuntime } from './local-runtime.js';
import { DispatchComposer } from './dispatch-composer.js';
import { createDispatchComposerState } from './dispatch-composer-state.js';
import type { DispatchComposerState } from './dispatch-composer-state.js';
import { useCopyReference } from './use-copy-reference.js';
import { useBoardProtection } from './board-navigation.js';

interface ShareProps {
  thread: BoardEntry;
  runtime: Pick<LocalRuntime, 'dispatch' | 'profiles' | 'rooms'>;
  close(): void;
}

/** A thread UUID is already understood by the native board reader; no new URI grammar. */
export function BoardReferenceActions({
  thread,
  ask,
}: {
  thread: BoardEntry;
  ask(trigger: HTMLButtonElement): void;
}) {
  const details = useRef<HTMLDetailsElement>(null);
  const { field, message, copy } = useCopyReference(thread.id, () => {
    if (details.current) details.current.open = true;
  });
  return (
    <section aria-label="Discussion actions">
      <div className="board-actions">
        <button
          type="button"
          onClick={(event) => ask(event.currentTarget)}
          disabled={thread.deleted}
        >
          Ask agents
        </button>
        <button type="button" onClick={() => void copy()}>
          Copy reference
        </button>
      </div>
      <details ref={details} className="board-reference">
        <summary>Thread reference</summary>
        <label>
          Local thread reference
          <input ref={field} readOnly value={thread.id} />
        </label>
        <p className="board-entry-meta">
          Live discussion, not a frozen snapshot. Copying sends nothing.
        </p>
      </details>
      {message && <p role="status">{message}</p>}
    </section>
  );
}

/** Resource-specific formatting only; the common composer owns audience, retries and receipts. */
export function BoardShare({ thread, runtime, close }: ShareProps) {
  const [state, setState] = useState<DispatchComposerState>();
  useEffect(() => {
    const owner = createDispatchComposerState(runtime.dispatch, {
      kind: 'request',
      message: (question) =>
        `${question}\n\nDiscussion thread: ${thread.id}\nRead: tmt office board show ${thread.id} --json\nThis is a live discussion reference, not a frozen snapshot. Read the current thread before replying.`,
    });
    setState(owner);
    return () => owner.dispose();
  }, [runtime.dispatch, thread.id]);
  return state ? (
    <BoardShareSession thread={thread} runtime={runtime} close={close} state={state} />
  ) : null;
}

function BoardShareSession({
  thread,
  runtime,
  close,
  state,
}: ShareProps & { state: DispatchComposerState }) {
  const back = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    back.current?.focus();
  }, []);
  const snapshot = useSyncExternalStore(state.subscribe, state.getSnapshot);
  const [dirty, setDirty] = useState(false);
  useBoardProtection(
    snapshot.busy || (!snapshot.receipt && (snapshot.attempted || snapshot.recoveryBlocked))
      ? 'pending'
      : dirty
        ? 'draft'
        : 'ready'
  );
  const [confirmClose, setConfirmClose] = useState(false);
  return (
    <section className="board-share" aria-label="Share discussion">
      <h2>{thread.title ?? 'Discussion'}</h2>
      <p>Ask agents about this live thread. Posting and copying alone never send a request.</p>
      <button
        type="button"
        ref={back}
        className="board-secondary"
        disabled={snapshot.busy}
        onClick={() => (dirty ? setConfirmClose(true) : close())}
      >
        Back to discussion
      </button>
      {confirmClose && (
        <div role="group" aria-label="Leave discussion request confirmation">
          <p>Discard this draft and return? This does not cancel any queued work.</p>
          <button type="button" disabled={snapshot.busy} onClick={close}>
            Discard and return
          </button>
          <button type="button" onClick={() => setConfirmClose(false)}>
            Keep composing
          </button>
        </div>
      )}
      <DispatchComposer
        state={state}
        profiles={runtime.profiles}
        rooms={runtime.rooms}
        onDraftChange={setDirty}
        copy={{
          label: 'Ask about discussion',
          heading: 'Ask agents',
          inputLabel: 'Question',
          placeholder: 'What should they review or contribute?',
          audienceHint: 'Only these agents will receive this request:',
        }}
      />
    </section>
  );
}
