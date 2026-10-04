import type { CommentContext } from './thread-store.js';
import { useEffect, useRef, useState } from 'react';
import type { PreviewAttempt } from './ask-preview.js';
import type { LedgerState } from './ask-records.js';
import { ReadRefusedError } from './ask-remote.js';
import type { RemoteAgent } from './ask-remote.js';
import type { AskDestination } from './ask-intent.js';
import { AskPreview } from './ask-preview.js';
import { text } from './strings.js';

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
  operationId: string;
  writer: string;
  message: string;
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

function presenceLabel(presence: RemoteAgent['presence']) {
  return presence === 'active'
    ? text.presenceOnline
    : presence === 'offline'
      ? text.presenceOffline
      : text.presenceUnknown;
}
function relativeTime(issuedAt: number, now: number) {
  if (now === 0) return text.askJustNow;
  const minutes = Math.round((issuedAt - now) / 60000);
  if (Math.abs(minutes) < 1) return text.askJustNow;
  const format = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
  if (Math.abs(minutes) < 60) return format.format(minutes, 'minute');
  const hours = Math.round(minutes / 60);
  return Math.abs(hours) < 24
    ? format.format(hours, 'hour')
    : format.format(Math.round(hours / 24), 'day');
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

export function AskControl({
  binding,
  selection,
  title,
  blocked,
  origin,
  originUnavailable = false,
}: {
  binding?: AskBinding;
  selection: string;
  title: string;
  blocked: boolean;
  origin?: { context: CommentContext; body: string };
  originUnavailable?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [quote, setQuote] = useState('');
  const [context, setContext] = useState<CommentContext>();
  const [comment, setComment] = useState('');
  const [agents, setAgents] = useState<(AskDestination & { presence?: RemoteAgent['presence'] })[]>(
    [],
  );
  const [agent, setAgent] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState<PreviewAttempt | null>(null);
  const generation = useRef(0);
  useEffect(() => {
    setOpen(false);
    setAttempt(null);
    const ref = generation;
    return () => {
      ref.current++;
    };
  }, [binding]);
  async function refresh() {
    const current = ++generation.current;
    setLoading(true);
    setError(null);
    setAgents([]);
    setAgent('');
    try {
      const destinations = await binding!.destinations();
      if (generation.current !== current) return;
      setAgents(destinations);
    } catch {
      if (generation.current === current) setError(text.askUnavailable);
    } finally {
      if (generation.current === current) setLoading(false);
    }
  }
  function close() {
    generation.current++;
    setOpen(false);
    setAttempt(null);
    setComment('');
    setLoading(false);
  }
  async function preview() {
    const destination = agents.find((value) => value.agent === agent);
    if (!binding || !destination || blocked) return;
    const current = ++generation.current;
    // Capture parent form values before any async authority check.
    const input = {
      quote,
      comment,
      title,
      url: location.href,
      destination: { ...destination },
      context,
    };
    setLoading(true);
    setError(null);
    try {
      const frozen = await binding.prepare(input);
      if (generation.current === current) setAttempt(frozen);
    } catch {
      if (generation.current === current) setError(text.askPreviewFailed);
    } finally {
      if (generation.current === current) setLoading(false);
    }
  }
  return (
    <div className="ask-control">
      <button
        data-testid={origin ? 'comment-ask-action' : 'ask-action'}
        disabled={open || !binding || (!selection && !origin) || blocked || originUnavailable}
        onClick={(event) => {
          if (!event.isTrusted) return;
          setContext(origin ? structuredClone(origin.context) : undefined);
          setComment(origin?.body ?? '');
          setQuote(selection);
          setOpen(true);
          setAttempt(null);
          void refresh();
        }}
      >
        {text.ask}
      </button>
      {!binding ? (
        <span className="isolation-note">{text.askUnavailable}</span>
      ) : !selection && !origin && !open ? (
        <span className="isolation-note">{text.askSelection}</span>
      ) : null}
      {open && !attempt && (
        <section className="ask-compose" aria-label={text.ask}>
          <h2>{text.ask}</h2>
          <p>{text.askVisible}</p>
          <blockquote>{quote}</blockquote>
          <label>
            {text.askComment}
            <textarea
              value={comment}
              disabled={loading || !!origin}
              onChange={(event) => setComment(event.target.value)}
            />
          </label>
          <fieldset data-testid="ask-agent-picker" disabled={loading || blocked}>
            <legend>{text.askPick}</legend>
            {agents.map((destination) => (
              <label
                key={destination.agent}
                className="ask-agent-option"
                data-testid="ask-agent-option"
                title={destination.agent}
                data-agent-id={destination.agent}
                data-delivery="unavailable"
              >
                <input
                  type="radio"
                  name="ask-agent"
                  value={destination.agent}
                  checked={agent === destination.agent}
                  onChange={() => setAgent(destination.agent)}
                />
                <span>
                  {destination.agentName}
                  <small>
                    {destination.machineName} · {presenceLabel(destination.presence)}
                  </small>
                </span>
              </label>
            ))}
          </fieldset>
          {loading && <p role="status">{text.askLoading}</p>}
          {!loading && !error && agents.length === 0 && <p role="status">{text.askNone}</p>}
          {blocked && <p role="status">{text.askOffline}</p>}
          {error && (
            <p role="alert" data-testid="ask-agents-unavailable">
              {error}
            </p>
          )}
          <div className="ask-actions">
            <button
              disabled={loading || blocked}
              onClick={(event) => {
                if (event.isTrusted) void refresh();
              }}
            >
              {text.askRefresh}
            </button>
            <button
              disabled={!agent || loading || blocked}
              onClick={(event) => {
                if (event.isTrusted) void preview();
              }}
            >
              {text.askPreview}
            </button>
            <button onClick={close}>{text.askClose}</button>
          </div>
        </section>
      )}
      {open && attempt && <AskPreview attempt={attempt} close={close} blocked={blocked} />}
    </div>
  );
}

/** Only admitted sync records enter this view. Responses to Send/retry/abandon
 * never create a page row or reply; the signed own stream remains its owner. */
export function AskPanel({
  records,
  binding,
  blocked,
}: {
  records: readonly PageAsk[];
  binding?: AskBinding;
  blocked: boolean;
}) {
  const [now, setNow] = useState(0);
  useEffect(() => setNow(Date.now()), [records]);
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
      <h2>{text.asks}</h2>
      <p>{text.askVisible}</p>
      {records.length === 0 && <p>{text.askEmpty}</p>}
      {records.map((record) => (
        <article
          data-testid="ask-entry"
          data-operation-id={record.operationId}
          data-writer={record.writer}
          key={`${record.writer}:${record.operationId}`}
          aria-label={`${text.ask} ${record.operationId}`}
        >
          <h3>{record.agentName || text.askAgentLabel}</h3>
          <p className="isolation-note">
            {record.deviceName} ·{' '}
            <time
              dateTime={new Date(record.issuedAt).toISOString()}
              title={`${text.askCreated} ${new Date(record.issuedAt).toISOString()}`}
            >
              {relativeTime(record.issuedAt, now)}
            </time>
          </p>
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
          <pre>{record.message}</pre>
          <p role="status" data-testid="ask-state" data-state={record.state}>
            {record.state === 'refused'
              ? (refusals[record.reason ?? ''] ?? states.refused)
              : record.state === 'accepted' &&
                  (record.reply !== undefined || record.resultUnavailable)
                ? text.askDeliveryAccepted
                : states[record.state]}
          </p>
          {record.state === 'uncertain' &&
            ['REMOTE_SESSION_ENDED', 'REMOTE_SEQUENCE_UNAVAILABLE'].includes(
              record.reason ?? '',
            ) && <p>{text.askSessionEnded}</p>}
          {['uncertain', 'held', 'dispatching', 'accepted'].includes(record.state) &&
            record.reply === undefined &&
            !record.resultUnavailable && (
              <>
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
              </>
            )}
          {errors.has(record.operationId) && <p role="alert">{errors.get(record.operationId)}</p>}
          {record.reply !== undefined && (
            <section aria-label={text.askReply}>
              <h4 data-testid="ask-reply-attribution">
                {text.askReplyFrom} {record.agentName || text.askAgentLabel}
                <small className="isolation-note">
                  {record.deviceName} · {relativeTime(record.issuedAt, now)}
                </small>
              </h4>
              <pre data-testid="ask-reply" data-empty={record.reply === ''}>
                {record.reply}
              </pre>
              {record.reply === '' && <p>{text.askEmptyReply}</p>}
            </section>
          )}
          {record.resultUnavailable && <p>{text.askFinalUnavailable}</p>}
        </article>
      ))}
    </section>
  );
}
