import type { CommentContext } from './thread-store.js';
import { CircleAlert, Clock, Pause } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { PreviewAttempt } from './ask-preview.js';
import { ASK_OBSERVATION_MS, type LedgerState } from './ask-records.js';
import { ReadRefusedError } from './ask-remote.js';
import type { RemoteAgent } from './ask-remote.js';
import type { AskDestination } from './ask-intent.js';
import { text } from './strings.js';
import { ConversationTurn } from './components/conversation-turn.js';

/** Capabilities stay in trusted parent chrome. The mounted adapter owns current
 * page/member/grant admission and returns the existing frozen/signing attempt. */
export interface AskBinding {
  destinations(): Promise<(AskDestination & { presence?: RemoteAgent['presence'] })[]>;
  prepare(input: {
    quote: string;
    comment: string;
    title: string;
    url: string;
    destination: AskDestination;
    context?: CommentContext;
  }): Promise<PreviewAttempt>;
  recheck(operationId: string): Promise<void>;
  abandon(operationId: string): Promise<void>;
}
export interface PageAsk {
  thread?: string;
  messageIds?: readonly string[];
  operationId: string;
  writer: string;
  message: string;
  deliveredMessage?: string;
  agent: string;
  agentName: string;
  deviceName: string;
  issuedAt: number;
  machine: string;
  state: LedgerState;
  canTrack: boolean;
  reason?: string | null;
  reply?: string;
  resultUnavailable?: boolean;
}

const refusals: Record<string, string> = {
  REMOTE_SCOPE_DENIED: text.askScopeDenied,
  REMOTE_INPUT_INVALID: text.askInputInvalid,
  REMOTE_RATE_LIMITED: text.askRateLimited,
  REMOTE_INTENT_CONFLICT: text.askIntentConflict,
  REMOTE_CLOSED: text.askRemoteClosed,
  REMOTE_SESSION_ENDED: text.askSessionEnded,
  REMOTE_INPUT_TOO_LARGE: text.askInputTooLarge,
  REMOTE_STATE_UNAVAILABLE: text.askStateUnavailable,
  REMOTE_CORE_UNAVAILABLE: text.askCoreUnavailable,
};

/** Only admitted sync records enter this view. Responses to Send/retry/abandon
 * never create a page row or reply; the signed own stream remains its owner. */
export function AskPanel({
  records,
  binding,
  blocked,
  inline = false,
  chat = false,
  renderUser,
}: {
  records: readonly PageAsk[];
  binding?: AskBinding;
  blocked: boolean;
  inline?: boolean;
  chat?: boolean;
  renderUser?(record: PageAsk, status: ReactNode, delivery: ReactNode): ReactNode;
}) {
  const [now, setNow] = useState(0);
  useEffect(() => {
    const timestamp = Date.now();
    setNow(timestamp);
    const deadlines = records
      .filter(
        (record) =>
          record.state === 'accepted' && record.reply === undefined && !record.resultUnavailable,
      )
      .map((record) => record.issuedAt + ASK_OBSERVATION_MS)
      .filter((deadline) => deadline > timestamp);
    if (!deadlines.length) return;
    const timer = setTimeout(() => setNow(Date.now()), Math.min(...deadlines) - timestamp);
    return () => clearTimeout(timer);
  }, [records, chat]);
  const [pending, setPending] = useState<Set<string>>(new Set());
  const [errors, setErrors] = useState<Map<string, string>>(new Map());
  const busy = useRef(new Set<string>());
  const active = useRef(true);
  const currentBinding = useRef(binding);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  useEffect(() => {
    currentBinding.current = binding;
    busy.current.clear();
    setPending(new Set());
    setErrors(new Map());
  }, [binding]);
  useEffect(() => setErrors(new Map()), [records]);
  async function action(record: PageAsk, kind: 'recheck' | 'abandon') {
    if (!binding || blocked || !record.canTrack || busy.current.has(record.operationId)) return;
    busy.current.add(record.operationId);
    setPending(new Set(busy.current));
    setErrors((previous) => new Map([...previous].filter(([id]) => id !== record.operationId)));
    try {
      await binding[kind](record.operationId);
    } catch (error) {
      const copy =
        error instanceof ReadRefusedError
          ? (refusals[error.code] ?? text.askActionFailed)
          : text.askActionFailed;
      if (active.current && currentBinding.current === binding)
        setErrors((previous) => new Map(previous).set(record.operationId, copy));
    } finally {
      if (active.current && currentBinding.current === binding) {
        busy.current.delete(record.operationId);
        setPending(new Set(busy.current));
      }
    }
  }
  const states = {
    dispatching: text.askDispatching,
    accepted: text.askAccepted,
    held: text.askHeld,
    uncertain: text.askUncertain,
    failed: text.askOperationFailed,
    refused: text.askRefused,
    cancelled: text.askCancelled,
    expired: text.askExpiredState,
    abandoned: text.askAbandoned,
  };

  return (
    <section className="ask-panel" aria-label={text.asks} data-testid="ask-panel">
      {!inline && !chat && (
        <>
          <h2>{text.asks}</h2>
          <p>{text.askVisible}</p>
        </>
      )}
      {!inline && !chat && records.length === 0 && <p>{text.askEmpty}</p>}
      {records.map((record) => {
        const timedOut =
          record.state === 'accepted' &&
          !record.resultUnavailable &&
          record.reply === undefined &&
          now >= record.issuedAt + ASK_OBSERVATION_MS;
        const stateCopy =
          record.state === 'refused'
            ? (refusals[record.reason ?? ''] ?? states.refused)
            : record.state === 'accepted' &&
                (record.reply !== undefined || record.resultUnavailable)
              ? text.askDeliveryAccepted
              : states[record.state];
        // Pending states are a mark plus a word: the mark is colored by the state's role, the word is
        // never dropped.
        const deliveryState: { mark: ReactNode; tone: string; label: string } = timedOut
          ? { mark: <Clock />, tone: 'waiting', label: 'no reply yet' }
          : ['dispatching', 'accepted'].includes(record.state) && !record.resultUnavailable
            ? { mark: <Clock />, tone: 'waiting', label: 'waiting' }
            : record.state === 'held'
              ? { mark: <Pause />, tone: 'held', label: `held · ${stateCopy}` }
              : { mark: <CircleAlert />, tone: 'problem', label: stateCopy };
        const status = record.reply === undefined && (
          <span role="status" data-testid="ask-state" data-state={record.state}>
            <span className="ask-state" data-tone={deliveryState.tone}>
              <span className="ask-state-mark" aria-hidden>
                {deliveryState.mark}
              </span>
              {deliveryState.label}
            </span>
          </span>
        );
        // Only the page-level ask list carries identities; no surface shows the delivered bytes on demand.
        const details =
          inline || chat ? null : (
            <details>
              <summary>{text.askDetails}</summary>
              <dl className="ask-identities">
                <dt>{text.askAgent}</dt>
                <dd>{record.agent}</dd>
                <dt>{text.askDeviceLabel}</dt>
                <dd>{record.writer}</dd>
                <dt>{text.askMachine}</dt>
                <dd>{record.machine}</dd>
                <dt>{text.askOperation}</dt>
                <dd>{record.operationId}</dd>
              </dl>
            </details>
          );
        const controls = (
          <>
            {record.state === 'uncertain' &&
              ['REMOTE_SESSION_ENDED', 'REMOTE_SEQUENCE_UNAVAILABLE'].includes(
                record.reason ?? '',
              ) && <p>{text.askSessionEnded}</p>}
            {['uncertain', 'held', 'dispatching', 'accepted'].includes(record.state) &&
              record.reply === undefined &&
              !record.resultUnavailable && (
                <div className="ask-actions">
                  <button
                    disabled={
                      !binding || blocked || !record.canTrack || pending.has(record.operationId)
                    }
                    onClick={(event) => {
                      if (event.isTrusted && record.canTrack) void action(record, 'recheck');
                    }}
                  >
                    {text.askRecheck}
                  </button>
                  {record.state === 'uncertain' && (
                    <button
                      disabled={
                        !binding || blocked || !record.canTrack || pending.has(record.operationId)
                      }
                      onClick={(event) => {
                        if (event.isTrusted) void action(record, 'abandon');
                      }}
                    >
                      {text.askAbandon}
                    </button>
                  )}
                </div>
              )}
            {errors.has(record.operationId) && <p role="alert">{errors.get(record.operationId)}</p>}
            {record.resultUnavailable && <p>{text.askFinalUnavailable}</p>}
          </>
        );
        const reply = record.reply !== undefined && (
          <ConversationTurn
            role="agent"
            layout={chat ? 'chat' : 'thread'}
            author={record.agentName || text.askAgentLabel}
            at={record.issuedAt}
            authorTitle={record.agent}
            bylineTestId="ask-reply-attribution"
            aria-label={text.askReply}
          >
            <pre data-testid="ask-reply" data-empty={record.reply === ''}>
              {record.reply}
            </pre>
            {record.reply === '' && <p>{text.askEmptyReply}</p>}
          </ConversationTurn>
        );
        const delivery = (
          <>
            {status}
            {details}
            {controls}
          </>
        );
        return (
          <div
            className="conversation-exchange"
            data-testid="ask-entry"
            data-operation-id={record.operationId}
            data-ledger-state={record.state}
            data-writer={record.writer}
            tabIndex={-1}
            key={`${record.writer}:${record.operationId}`}
            aria-label={`${text.ask} ${record.operationId}`}
          >
            {renderUser ? (
              renderUser(
                record,
                status,
                <>
                  {details}
                  {controls}
                </>,
              )
            ) : (
              <ConversationTurn
                role="user"
                layout={chat ? 'chat' : 'thread'}
                author={record.deviceName || text.commentDevice}
                at={record.issuedAt}
                authorTitle={record.writer}
                delivery={delivery}
              >
                {!inline && <pre>{record.message}</pre>}
              </ConversationTurn>
            )}
            {reply}
          </div>
        );
      })}
    </section>
  );
}
