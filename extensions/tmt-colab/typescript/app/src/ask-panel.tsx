import { AgentActivityLine } from './components/agent-activity.js';
import { ConversationPresentation } from './components/conversation-presentation.js';
import type { AnswerBatchContext, CommentContext } from './thread-store.js';
import { BrowserIconAction, BusyMark, useBusyMark } from '@tmt/browser-ui/react';
import { CircleAlert, Clock, Pause, RotateCw } from 'lucide-react';
import { useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import type { PreviewAttempt } from './ask-preview.js';
import { sameMessageIds, type LedgerState } from './ask-records.js';
import { askedMessageIds } from './answer-batch.js';
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
import type { SendIdentityNotice } from './send-identity-notice.js';
import { text } from './strings.js';
import { agentSentence } from './components/agent-chip.js';
import { MarkdownText } from './components/markdown-text.js';
import { ConversationTurn } from './components/conversation-turn.js';
import { conversationRecordKey } from './conversation-record-key.js';

/** Capabilities stay in trusted parent chrome. The mounted adapter owns current
 * page/member/grant admission and returns the existing frozen/signing attempt. */
export interface AskBinding {
  /** How Remote attributed this page session's first send worth saying; absent when none. */
  identity?: SendIdentityNotice;
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
    /** Several answer comments of one question group, asked in one Ask (#2875); `context` is the
     * first of them, the batch's canonical thread. */
    batch?: AnswerBatchContext;
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

/** Presentation chooses a verified retry context for each operation; the binding rechecks it. */
export type AskRetryInput = AskAgainInput | ((record: PageAsk) => AskAgainInput | undefined);

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

/** A dispatching ask is a sent message in flight: its mark has no words and follows the shared
 * busy-mark delay, as a sent message's does, so a fast dispatch changes nothing visible. */
function DispatchingMark() {
  const marked = useBusyMark(true);
  return (
    <span className="comment-echo-mark" aria-hidden="true" data-shown={marked || undefined}>
      <BusyMark />
    </span>
  );
}

/** Only admitted sync records enter this view. Responses to Send/retry/abandon
 * never create a page row or reply; the signed own stream remains its owner. */
export function AskPanel({
  records,
  binding,
  blocked,
  readOnly = false,
  inline = false,
  dayHeadings,
  renderUser,
  retryInput: suppliedRetryInput,
}: {
  records: readonly PageAsk[];
  binding?: AskBinding;
  blocked: boolean;
  readOnly?: boolean;
  inline?: boolean;
  dayHeadings?: readonly ReactNode[];
  renderUser?(record: PageAsk, status: ReactNode, delivery: ReactNode): ReactNode;
  retryInput?: AskRetryInput;
}) {
  const chat = useContext(ConversationPresentation) === 'chat';
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
    const retryInput =
      typeof suppliedRetryInput === 'function' ? suppliedRetryInput(record) : suppliedRetryInput;
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
    const deliveryState: { mark: ReactNode; tone: string; label: ReactNode } = waitingForAgent
      ? { mark: <Clock />, tone: 'waiting', label: agentSentence(text.askWaiting, agent) }
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
    // The retry is of exactly the comments that Ask was about: one answer, or the whole batch.
    const retryable =
      retryInput !== undefined &&
      sameMessageIds(record.messageIds ?? [], askedMessageIds(retryInput));
    const canAskAgain =
      retryable &&
      retryInput.context.message.writer === record.writer &&
      record.canTrack &&
      recipientRecords.every(provablyUnsent) &&
      recipientRecords.at(-1) === record;
    const status = record.reply === undefined && (
      <span className="conversation-status" aria-label={`Ask ${agent}`}>
        <span role="status" data-testid="ask-state" data-state={record.state}>
          <span className="ask-state" data-tone={deliveryState.tone}>
            {!(
              (record.state === 'accepted' && !record.resultUnavailable) ||
              record.state === 'dispatching'
            ) && (
              <span className="ask-state-mark" aria-hidden>
                {chat && deliveryState.label === text.askNotDelivered ? '✗' : deliveryState.mark}
              </span>
            )}
            {record.state === 'accepted' && !record.resultUnavailable ? (
              <AgentActivityLine record={record} />
            ) : record.state === 'dispatching' ? (
              <DispatchingMark />
            ) : (
              <>
                {records.length > 1 && !waitingForAgent
                  ? agentSentence(text.askRecipientPrefix, agent)
                  : ''}
                {deliveryState.label}
              </>
            )}
          </span>
        </span>
        {!readOnly && canAskAgain && (
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
        {!readOnly && canRecheck && (
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
            {records.length > 1 ? agentSentence(text.askRecipientPrefix, agent) : ''}
            {supporting}
          </p>
        )}
        {record.reply === undefined &&
          record.state === 'refused' &&
          REMOTE_REFUSAL_CODES.some((code) => code === record.reason) &&
          !retryable && (
            <p className="ask-supporting">{agentSentence(text.messageAskAgain, agent)}</p>
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
        data-conversation-record={conversationRecordKey('reply', record.writer, record.operationId)}
        author={record.agentName || text.askAgentLabel}
        at={record.issuedAt}
        authorTitle={record.agent}
        bylineTestId="ask-reply-attribution"
        aria-label={text.askReply}
      >
        <MarkdownText
          data-testid="ask-reply"
          data-empty={record.reply === ''}
          value={record.reply}
          names={[record.agentName]}
        />
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
            {dayHeadings?.[index]}
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
                <MarkdownText value={record.message} names={[record.agentName]} />
              </ConversationTurn>
            )}
            {reply}
          </div>
        );
      })}
    </section>
  );
}
