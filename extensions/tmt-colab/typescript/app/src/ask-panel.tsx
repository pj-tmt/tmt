import type { CommentContext } from './thread-store.js';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { PreviewAttempt } from './ask-preview.js';
import { ASK_OBSERVATION_MS, type LedgerState } from './ask-records.js';
import { ReadRefusedError } from './ask-remote.js';
import type { RemoteAgent } from './ask-remote.js';
import type { AskDestination } from './ask-intent.js';
import { text } from './strings.js';
import { relativeTime, compactRelativeTime } from './display-time.js';

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
    if (!chat) return;
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
        const status = (
          <span role="status" data-testid="ask-state" data-state={record.state}>
            {chat
              ? record.reply !== undefined
                ? '✓ replied'
                : timedOut
                  ? '… no reply yet'
                  : ['dispatching', 'accepted'].includes(record.state) && !record.resultUnavailable
                    ? '… waiting'
                    : record.state === 'held'
                      ? `• held · ${stateCopy}`
                      : `! ${stateCopy}`
              : stateCopy}
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
          <section className={chat ? 'chat-agent-turn' : undefined} aria-label={text.askReply}>
            {chat ? (
              <p className="chat-byline" data-testid="ask-reply-attribution">
                {record.agentName || text.askAgentLabel} ·{' '}
                <time dateTime={new Date(record.issuedAt).toISOString()}>
                  {compactRelativeTime(record.issuedAt, now)}
                </time>
              </p>
            ) : (
              <h4 data-testid="ask-reply-attribution">
                {text.askReplyFrom} {record.agentName || text.askAgentLabel}
                <small className="isolation-note">
                  {record.deviceName} · {relativeTime(record.issuedAt, now)}
                </small>
              </h4>
            )}
            <pre data-testid="ask-reply" data-empty={record.reply === ''}>
              {record.reply}
            </pre>
            {record.reply === '' && <p>{text.askEmptyReply}</p>}
          </section>
        );
        return (
          <article
            className={chat ? 'chat-exchange' : undefined}
            data-testid="ask-entry"
            data-operation-id={record.operationId}
            data-writer={record.writer}
            tabIndex={-1}
            key={`${record.writer}:${record.operationId}`}
            aria-label={`${text.ask} ${record.operationId}`}
          >
            {chat ? (
              <>
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
                  <article className="chat-user-turn">
                    <header>
                      <p className="chat-byline">
                        {record.deviceName} · {compactRelativeTime(record.issuedAt, now)}
                      </p>
                      {status}
                    </header>
                    <pre>{record.message}</pre>
                    {details}
                    {controls}
                  </article>
                )}
                {reply}
              </>
            ) : (
              <>
                {!inline && <h3>{record.agentName || text.askAgentLabel}</h3>}
                <p className="isolation-note">
                  {inline ? record.agentName || text.askAgentLabel : record.deviceName} ·{' '}
                  <time
                    dateTime={new Date(record.issuedAt).toISOString()}
                    title={`${text.askCreated} ${new Date(record.issuedAt).toISOString()}`}
                  >
                    {relativeTime(record.issuedAt, now)}
                  </time>
                </p>
                {details}
                {!inline && <pre>{record.message}</pre>}
                {status}
                {controls}
                {reply}
              </>
            )}
          </article>
        );
      })}
    </section>
  );
}
