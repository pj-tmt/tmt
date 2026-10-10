import type { CommentContext } from './thread-store.js';
import { BrowserIconAction } from '@tmt/browser-ui/react';
import { CircleAlert, Clock, Pause, RotateCw } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { PreviewAttempt } from './ask-preview.js';
import { ASK_OBSERVATION_MS, type LedgerState } from './ask-records.js';
import {
  ReadRefusedError,
  REMOTE_REFUSAL_CODES,
  TALK_NOT_ENABLED,
  provablyUnsent,
} from './ask-remote.js';
import { AskAgainAction, type AskAgainInput } from './ask-again.js';
import type { RemoteAgent } from './ask-remote.js';
import type { AskDestination } from './ask-intent.js';
import type { AgentDirectoryObservation } from './live-ask.js';
import { text } from './strings.js';
import { MessageText } from './components/message-text.js';
import { ConversationTurn } from './components/conversation-turn.js';

/** Capabilities stay in trusted parent chrome. The mounted adapter owns current
 * page/member/grant admission and returns the existing frozen/signing attempt. */
export interface AskBinding {
  /** Status-only read; never the effect-bearing Ask destination admission path. */
  observeDestinations?(): Promise<AgentDirectoryObservation>;
  destinations(): Promise<(AskDestination & { presence?: RemoteAgent['presence'] })[]>;
  prepare(input: {
    quote: string;
    comment: string;
    title: string;
    url: string;
    destination: AskDestination;
    context?: CommentContext;
    /** Absent for Send; null for a trusted local pre-adoption failure; otherwise
     * the own refused operation. The store rechecks the recipient before adoption. */
    retryOf?: string | null;
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
  requestId?: string | null;
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
  renderUser,
  retryInput,
}: {
  records: readonly PageAsk[];
  binding?: AskBinding;
  blocked: boolean;
  inline?: boolean;
  renderUser?(record: PageAsk, status: ReactNode, delivery: ReactNode): ReactNode;
  retryInput?: AskAgainInput;
}) {
  const [now, setNow] = useState(0);
  useEffect(() => {
    const deadlines = records
      .filter(
        (record) =>
          record.state === 'accepted' && record.reply === undefined && !record.resultUnavailable,
      )
      .map((record) => record.issuedAt + ASK_OBSERVATION_MS);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const update = () => {
      const timestamp = Date.now();
      setNow(timestamp);
      const future = deadlines.filter((deadline) => deadline > timestamp);
      // Historical intent times can exceed the browser's signed 32-bit timer delay.
      if (future.length)
        timer = setTimeout(update, Math.min(2_147_483_647, Math.min(...future) - timestamp));
    };
    update();
    return () => clearTimeout(timer);
  }, [records]);
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

  const turns = records.map((record) => {
    const timedOut =
      record.state === 'accepted' &&
      !record.resultUnavailable &&
      record.reply === undefined &&
      now >= record.issuedAt + ASK_OBSERVATION_MS;
    const stateCopy =
      record.state === 'refused'
        ? record.reason === TALK_NOT_ENABLED
          ? text.askTalkNotEnabled(record.writer)
          : (refusals[record.reason ?? ''] ?? states.refused)
        : record.state === 'accepted' && (record.reply !== undefined || record.resultUnavailable)
          ? text.askDeliveryAccepted
          : states[record.state];
    const agent = record.agentName || text.askAgentLabel;
    const waitingForAgent =
      ['dispatching', 'accepted'].includes(record.state) && !record.resultUnavailable;
    const deliveryState: { mark: ReactNode; tone: string; label: string } = timedOut
      ? { mark: <Clock />, tone: 'waiting', label: text.askNoReply(agent) }
      : waitingForAgent
        ? { mark: <Clock />, tone: 'waiting', label: text.askWaiting(agent) }
        : record.state === 'held'
          ? { mark: <Pause />, tone: 'held', label: text.askApproval }
          : {
              mark: <CircleAlert />,
              tone: 'problem',
              label: record.resultUnavailable
                ? text.askResultUnavailable
                : record.state === 'uncertain'
                  ? text.askUnconfirmed
                  : record.state === 'abandoned'
                    ? text.askTrackingAbandoned
                    : text.askNotDelivered,
            };
    const canRecheck =
      ['uncertain', 'held', 'dispatching', 'accepted'].includes(record.state) &&
      record.reply === undefined &&
      !record.resultUnavailable;
    const disabled = !binding || blocked || !record.canTrack || pending.has(record.operationId);
    const recipientRecords = records.filter(
      (value) =>
        value.writer === record.writer &&
        value.machine === record.machine &&
        value.agent === record.agent,
    );
    const canAskAgain =
      !!retryInput &&
      record.messageIds?.length === 1 &&
      retryInput.context.message.writer === record.writer &&
      record.canTrack &&
      recipientRecords.every(provablyUnsent) &&
      recipientRecords.at(-1) === record;
    const status = record.reply === undefined && (
      <span className="conversation-status" aria-label={`Ask ${agent}`}>
        <span role="status" data-testid="ask-state" data-state={record.state}>
          <span className="ask-state" data-tone={deliveryState.tone}>
            <span className="ask-state-mark" aria-hidden>
              {deliveryState.mark}
            </span>
            {records.length > 1 && !timedOut && !waitingForAgent
              ? text.askRecipientPrefix(agent)
              : ''}
            {deliveryState.label}
          </span>
        </span>
        {canAskAgain && (
          <AskAgainAction
            key={recipientRecords
              .map((value) => value.operationId)
              .sort()
              .join(':')}
            binding={binding}
            blocked={disabled}
            input={retryInput!}
            recipient={record}
            retryOf={record.operationId}
          />
        )}
        {canRecheck && (
          <>
            <BrowserIconAction
              type="button"
              variant="text"
              label={text.askRecheck}
              icon={<RotateCw />}
              disabled={disabled}
              onActivate={(event) => {
                if (event.isTrusted && record.canTrack) void action(record, 'recheck');
              }}
            />
            {record.state === 'uncertain' && (
              <button
                className="ask-status-action"
                disabled={disabled}
                onClick={(event) => {
                  if (event.isTrusted) void action(record, 'abandon');
                }}
              >
                {text.askAbandon}
              </button>
            )}
          </>
        )}
      </span>
    );
    // Only the page-level ask list carries identities; no surface shows the delivered bytes on demand.
    const details = inline ? null : (
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
    const supporting = record.resultUnavailable
      ? `${text.askDeliveryAccepted} ${text.askFinalUnavailable}`
      : record.reply === undefined && !['dispatching', 'accepted'].includes(record.state)
        ? stateCopy
        : undefined;
    const controls = (
      <>
        {supporting && (
          <p className="ask-supporting">
            {records.length > 1 ? text.askRecipientPrefix(agent) : ''}
            {supporting}
          </p>
        )}
        {record.reply === undefined &&
          record.state === 'refused' &&
          REMOTE_REFUSAL_CODES.some((code) => code === record.reason) &&
          (!retryInput || record.messageIds?.length !== 1) && (
            <p className="ask-supporting">{text.messageAskAgain(agent)}</p>
          )}
        {record.state === 'uncertain' &&
          ['REMOTE_SESSION_ENDED', 'REMOTE_SEQUENCE_UNAVAILABLE'].includes(record.reason ?? '') && (
            <p className="ask-supporting">{text.askSessionEnded}</p>
          )}
        {errors.has(record.operationId) && <p role="alert">{errors.get(record.operationId)}</p>}
      </>
    );
    const reply = record.reply !== undefined && (
      <ConversationTurn
        role="agent"
        author={record.agentName || text.askAgentLabel}
        at={record.issuedAt}
        authorTitle={record.agent}
        bylineTestId="ask-reply-attribution"
        aria-label={text.askReply}
      >
        <pre data-testid="ask-reply" data-empty={record.reply === ''}>
          <MessageText value={record.reply} names={[record.agentName]} />
        </pre>
        {record.reply === '' && <p>{text.askEmptyReply}</p>}
      </ConversationTurn>
    );
    const delivery = (
      <>
        {controls}
        {details}
      </>
    );
    return { record, status, delivery, reply };
  });
  return (
    <section className="ask-panel" aria-label={text.asks} data-testid="ask-panel">
      {!inline && (
        <>
          <h2>{text.asks}</h2>
          <p>{text.askVisible}</p>
        </>
      )}
      {!inline && records.length === 0 && <p>{text.askEmpty}</p>}
      {turns.map(({ record, status, delivery, reply }, index) => {
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
              index === 0 &&
              renderUser(
                record,
                turns.some((turn) => turn.status) ? (
                  <>
                    {turns.map((turn) =>
                      turn.status ? <span key={turn.record.operationId}>{turn.status}</span> : null,
                    )}
                  </>
                ) : undefined,
                <>
                  {turns.map((turn) => (
                    <div key={turn.record.operationId}>{turn.delivery}</div>
                  ))}
                </>,
              )
            ) : (
              <ConversationTurn
                role="user"
                author={record.deviceName || text.commentDevice}
                at={record.issuedAt}
                authorTitle={record.writer}
                status={status}
                delivery={delivery}
              >
                <pre>
                  <MessageText value={record.message} names={[record.agentName]} />
                </pre>
              </ConversationTurn>
            )}
            {reply}
          </div>
        );
      })}
    </section>
  );
}
