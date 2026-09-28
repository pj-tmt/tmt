import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import type { IdentityChoice } from '../profiles/identity-choice.js';
import type { LocalRuntime } from './local-runtime.js';
import { createConversationState } from './conversation-state.js';
import type { ConversationState } from './conversation-state.js';
import { createDispatchJournal } from './dispatch-journal.js';
import { createDispatchComposerState } from './dispatch-composer-state.js';
import type { DispatchComposerState } from './dispatch-composer-state.js';
import type { HistoryItem, HistoryDetail } from './request-history-contract.js';
import { conversationCue } from './conversation-cue.js';
import type { ConversationCue } from './conversation-cue.js';
import './agent-conversation.css';

type Runtime = Pick<LocalRuntime, 'dispatch' | 'requests'>;
export interface ConversationTarget extends IdentityChoice {
  room?: { id: string; name: string };
  areaId?: string;
}

interface Props {
  refreshHost?: HTMLElement | null;
  target: ConversationTarget;
  runtime: Runtime;
  active: boolean;
  onDirtyChange?(dirty: boolean): void;
  reading?: boolean;
  minimized?: boolean;
  compact?: boolean;
  onCueChange?(cue: ConversationCue): void;
}
export function AgentConversation(props: Props) {
  const { target, runtime } = props;
  const { id, name } = target;
  const roomId = target.room?.id;
  const [owners, setOwners] = useState<{
    history: ConversationState;
    composer: DispatchComposerState;
  }>();
  useEffect(() => {
    const history = createConversationState(runtime.requests, {
      recipientId: id,
      ...(roomId ? { roomId } : {}),
    });
    const composer = createDispatchComposerState(
      runtime.dispatch,
      {
        kind: 'request',
        message: (text) => text,
        recipients: [{ id, name }],
        ...(roomId ? { context: { kind: 'direct' as const, roomId } } : {}),
      },
      undefined,
      {
        journal: createDispatchJournal({ kind: 'direct', recipientId: id, roomId }),
        lookup: (id, signal) => runtime.requests.receipt(id, signal),
      }
    );
    setOwners({ history, composer });
    return () => {
      history.dispose();
      composer.dispose();
    };
  }, [runtime.dispatch, runtime.requests, id, name, roomId]);
  return owners ? (
    <ConversationSession {...props} {...owners} />
  ) : (
    <p role="status">Opening conversation…</p>
  );
}

function deliveryLabel(item: HistoryItem<unknown>): string {
  if (item.delivery === 'definitely_failed') return 'Not delivered';
  if (item.delivery === 'uncertain') return 'Delivery uncertain';
  return item.recipientAcknowledged ? 'Acknowledged' : 'Sent';
}

function Exchange({ detail, name }: { detail: HistoryDetail; name: string }) {
  return (
    <>
      <div className="chat-message chat-outgoing">
        <span className="chat-speaker">
          {detail.sender.kind === 'unknown'
            ? 'Local sender'
            : `Identity ${detail.sender.identityId}`}
        </span>
        {detail.prompt.status === 'retained' ? (
          <pre>{detail.prompt.message}</pre>
        ) : (
          <p>Message {detail.prompt.status}.</p>
        )}
        <span className="chat-delivery">{deliveryLabel(detail)}</span>
      </div>
      <div className="chat-message chat-incoming">
        <span className="chat-speaker">{name}</span>
        {detail.final.status === 'retained' ? (
          <pre className="conversation-reply">{detail.final.content}</pre>
        ) : (
          <p className="chat-waiting">
            {detail.final.status === 'not_submitted'
              ? detail.delivery === 'definitely_failed'
                ? 'This message was not delivered.'
                : 'Waiting for a reply'
              : detail.final.status === 'not_required'
                ? 'No reply requested'
                : `Reply ${detail.final.status}`}
          </p>
        )}
      </div>
    </>
  );
}

function ConversationSession({
  target,
  active,
  onDirtyChange,
  reading = true,
  minimized = false,
  compact = false,
  refreshHost,
  onCueChange,
  history,
  composer,
}: Props & { history: ConversationState; composer: DispatchComposerState }) {
  const snapshot = useSyncExternalStore(history.subscribe, history.getSnapshot);
  const composition = useSyncExternalStore(composer.subscribe, composer.getSnapshot);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [submittedId, setSubmittedId] = useState<string>();
  const [seenReplyIds, setSeenReplyIds] = useState<readonly string[]>([]);
  const feed = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const followLatest = useRef(true);
  const dirty = !composition.receipt && Boolean(composition.text || composition.recoveryBlocked);
  useEffect(() => {
    const sync = () => {
      history.setActive(active && !document.hidden);
      if (active && !document.hidden) void composer.recover();
    };
    sync();
    document.addEventListener('visibilitychange', sync);
    return () => {
      document.removeEventListener('visibilitychange', sync);
      history.setActive(false);
    };
  }, [active, history, composer]);
  useEffect(() => {
    onDirtyChange?.(dirty);
    if (!dirty) return;
    const warn = (event: BeforeUnloadEvent) => event.preventDefault();
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [dirty, onDirtyChange]);
  useEffect(() => {
    if (!composition.receipt) return;
    setSubmittedId(composition.receipt.items[0]?.requestId);
    followLatest.current = true;
    history.latest();
    composer.discard();
    if (reading) input.current?.focus();
  }, [composition.receipt, history, composer, reading]);
  useEffect(() => {
    if (minimized) history.latest();
  }, [minimized, history]);
  const cue = useMemo(
    () => conversationCue(snapshot, submittedId, seenReplyIds),
    [snapshot, submittedId, seenReplyIds]
  );
  useEffect(() => {
    onCueChange?.(cue);
  }, [cue, onCueChange]);
  useEffect(() => {
    if (!reading || document.hidden) return;
    // Only loaded replies in this bounded view count as seen; this is not inbox ack.
    const ids = (snapshot.page?.items ?? [])
      .filter((item) => snapshot.details[item.requestId]?.final.status === 'retained')
      .map((item) => item.requestId);
    setSeenReplyIds((previous) =>
      previous.length === ids.length && previous.every((id, index) => id === ids[index])
        ? previous
        : ids
    );
  }, [reading, snapshot.page, snapshot.details]);
  useEffect(() => {
    const element = feed.current;
    if (element && followLatest.current && !snapshot.before)
      element.scrollTop = element.scrollHeight;
  }, [snapshot.page, snapshot.details, snapshot.before]);
  function send() {
    if (composition.review) {
      void (async () => {
        await composer.recover();
        if (!composer.getSnapshot().receipt) await composer.send();
      })();
    } else {
      // Freeze exact intent internally; a direct message needs no second review screen.
      composer.review();
      void composer.send();
    }
  }
  const messages = [...(snapshot.page?.items ?? [])].reverse();
  return (
    <section
      className="agent-conversation"
      data-empty={messages.length === 0}
      aria-label={`Messages with ${target.name}`}
    >
      <header className="conversation-heading">
        {!compact && (
          <>
            <span className="conversation-eyebrow">DIRECT MESSAGE</span>
            <h2>{target.name}</h2>
            <p>Delivered through TMT inbox</p>
          </>
        )}
        {target.room && (
          <p className="conversation-room">
            Room: {target.room.name}. Only {target.name} receives this message.
          </p>
        )}
      </header>
      <div
        className="chat-toolbar"
        hidden={Boolean(refreshHost) && !snapshot.page?.nextBefore && !snapshot.before}
      >
        {snapshot.page?.nextBefore && (
          <button
            disabled={snapshot.loading}
            onClick={() => {
              followLatest.current = false;
              history.older();
            }}
          >
            Earlier messages
          </button>
        )}
        {snapshot.before && (
          <button
            onClick={() => {
              followLatest.current = true;
              history.latest();
            }}
          >
            Latest messages
          </button>
        )}
        {refreshHost ? (
          createPortal(
            <button
              aria-label="Refresh"
              title="Refresh conversation"
              disabled={snapshot.loading}
              onClick={() => history.refresh()}
            >
              ↻
            </button>,
            refreshHost
          )
        ) : (
          <button disabled={snapshot.loading} onClick={() => history.refresh()}>
            Refresh
          </button>
        )}
      </div>
      {snapshot.error && (
        <p className="chat-notice" role="alert">
          {snapshot.error}
        </p>
      )}
      {snapshot.paused && (
        <p className="chat-notice" role="status">
          Updates paused. Refresh to resume.
        </p>
      )}
      <div
        ref={feed}
        className="chat-feed"
        role="log"
        aria-label={`Conversation with ${target.name}`}
        aria-live="polite"
        onScroll={() => {
          const element = feed.current;
          if (element)
            followLatest.current =
              element.scrollHeight - element.scrollTop - element.clientHeight < 64;
        }}
      >
        {messages.length === 0 && (
          <p className="chat-empty">
            {snapshot.page
              ? `Start a conversation with ${target.name}.`
              : snapshot.error
                ? 'History could not be loaded.'
                : 'Loading conversation…'}
          </p>
        )}
        {messages.map((item) => (
          <article key={item.requestId} className="chat-exchange" data-request-id={item.requestId}>
            {snapshot.details[item.requestId] ? (
              <Exchange detail={snapshot.details[item.requestId]!} name={target.name} />
            ) : (
              <p className="chat-notice">
                {snapshot.detailErrors[item.requestId] ?? 'Loading message…'}
              </p>
            )}
          </article>
        ))}
      </div>
      <form
        className="conversation-compose"
        onSubmit={(event) => {
          event.preventDefault();
          send();
        }}
      >
        {composition.error && <p role="alert">{composition.error}</p>}
        <label className="chat-input-label">
          <textarea
            ref={input}
            aria-label="Message"
            value={composition.review?.message ?? composition.text}
            readOnly={Boolean(composition.review)}
            disabled={composition.recoveryBlocked || Boolean(composition.receipt)}
            placeholder={`Message ${target.name}…`}
            onChange={(event) => composer.change(event.target.value, [target])}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                send();
              }
            }}
          />
        </label>
        <div className="chat-compose-actions">
          <span className="conversation-tip">Enter to send · Shift + Enter for a new line</span>
          {composition.receipt ? (
            <button type="button" onClick={() => composer.discard()}>
              New message
            </button>
          ) : (
            <button
              className="conversation-primary"
              type="submit"
              disabled={
                composition.busy ||
                composition.recoveryBlocked ||
                composition.rejected ||
                !composition.text.trim()
              }
            >
              {composition.busy ? 'Sending…' : composition.attempted ? 'Retry' : 'Send'}
            </button>
          )}
        </div>
        {dirty &&
          (composition.review || composition.recoveryBlocked) &&
          !composition.busy &&
          (confirmDiscard ? (
            <div className="chat-discard" role="group" aria-label="Discard message confirmation">
              <p>Discard this local message? This does not cancel work already sent.</p>
              <button
                type="button"
                onClick={() => {
                  composer.discard();
                  setConfirmDiscard(false);
                }}
              >
                Confirm discard
              </button>
              <button type="button" onClick={() => setConfirmDiscard(false)}>
                Keep message
              </button>
            </div>
          ) : (
            <button
              className="chat-discard-toggle"
              type="button"
              onClick={() => setConfirmDiscard(true)}
            >
              Discard draft
            </button>
          ))}
        <details className="conversation-tip">
          <summary>How delivery works</summary>
          Agents receive these messages through <code>tmt x listen --identity {target.id}</code>,
          not terminal input. Sent does not mean read or completed. Unconfirmed sends stay in this
          browser tab for safe retry.
        </details>
      </form>
    </section>
  );
}
