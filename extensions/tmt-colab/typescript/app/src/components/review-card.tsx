import { BrowserAction } from '@tmt/browser-ui/react';
import { useEffect, useId, useState, type ReactNode } from 'react';
import type { Review } from '../questions.js';
import type { ReviewOutcome } from '../review-actions.js';
import type { ReviewState } from '../reviews.js';
import { REVIEW_CHANGE_NOTE_BYTES, REVIEW_CONFIRM_NOTE_POINTS } from '../reviews.js';
import { compactRelativeTime } from '../display-time.js';
import { text } from '../strings.js';
import { AgentChip, AgentList, agentSentence } from './agent-chip.js';
import { NeedsYouBar } from './needs-you-bar.js';

/** Authenticated parent presentation of one review section. The callbacks own every effect; the
 * page's own HTML cannot render, press or fake any of this. */
export function ReviewCard({
  review,
  state,
  outcome,
  recipient,
  disabled,
  readOnly = false,
  confirm,
  requestChange,
  reopen,
  children,
}: {
  review: Review;
  state: ReviewState;
  outcome?: ReviewOutcome;
  /** The asking agent that will be told, or undefined when the comment stays on the page. */
  recipient?: string;
  disabled: boolean;
  readOnly?: boolean;
  confirm(note: string): void;
  requestChange(note: string): void;
  reopen(): void;
  children?: ReactNode;
}) {
  const id = useId();
  const [noting, setNoting] = useState(false);
  const [note, setNote] = useState('');
  const [changing, setChanging] = useState(false);
  const [change, setChange] = useState('');
  const [now, setNow] = useState(0);
  const closedAt = state.kind === 'closed' ? state.at : undefined;
  useEffect(() => setNow(Date.now()), [closedAt]);
  // A settled action empties its fields; a failed one keeps them for the next try.
  useEffect(() => {
    if (outcome?.state !== 'settled') return;
    setNoting(false);
    setNote('');
    setChanging(false);
    setChange('');
  }, [outcome]);
  const working = outcome?.state === 'working';
  const busy = disabled || working;
  const closed = state.kind === 'closed';
  const action = (label: string, onActivate: () => void, options: { disabled?: boolean } = {}) => (
    <BrowserAction
      key={label}
      type="button"
      variant="text"
      label={label}
      disabled={options.disabled ?? busy}
      onActivate={(event) => {
        if (event.isTrusted) onActivate();
      }}
    />
  );
  const actions = readOnly
    ? undefined
    : closed
      ? action(text.reviewReopen, reopen)
      : changing
        ? [
            action(text.reviewChangeSend, () => requestChange(change), {
              disabled: busy || !change.trim(),
            }),
            action(
              text.reviewCancel,
              () => {
                setChanging(false);
                setChange('');
              },
              { disabled: working },
            ),
          ]
        : [
            action(text.reviewConfirm, () => confirm(noting ? note : '')),
            !noting && action(text.reviewAddNote, () => setNoting(true)),
            action(text.reviewNeedsChange, () => {
              setNoting(false);
              setNote('');
              setChanging(true);
            }),
          ];
  const closedAgo =
    state.kind === 'closed' && now && state.at && Number.isFinite(Number(state.at))
      ? compactRelativeTime(Number(state.at), now)
      : undefined;
  return (
    <section
      className="review-card"
      aria-label={text.reviewSection(review.title)}
      data-review-id={review.id}
      data-review-state={state.kind}
    >
      {state.kind === 'closed' ? (
        <NeedsYouBar
          tone="done"
          word={text.reviewClosed}
          who={state.by}
          why={[closedAgo, state.note && `“${state.note}”`].filter(Boolean).join(' · ')}
          actions={actions}
        />
      ) : state.kind === 'changes' ? (
        <NeedsYouBar
          tone="agent"
          word={text.reviewChanges}
          why={agentSentence(text.reviewWaitingFor, review.agent)}
          actions={actions}
        />
      ) : (
        <NeedsYouBar
          tone="needs"
          word={text.reviewAsk}
          who={<AgentChip name={review.agent} />}
          why={review.title}
          actions={actions}
        />
      )}
      <div className="review-body">
        {state.kind === 'changes' && state.note && <p className="review-note">{state.note}</p>}
        {!readOnly && !closed && noting && (
          <div className="review-field">
            <label htmlFor={`${id}-note`}>{text.reviewNoteLabel}</label>
            <input
              id={`${id}-note`}
              value={note}
              disabled={busy}
              maxLength={REVIEW_CONFIRM_NOTE_POINTS * 2}
              onChange={(event) => setNote(event.target.value)}
            />
          </div>
        )}
        {!readOnly && !closed && changing && (
          <div className="review-field">
            <label htmlFor={`${id}-change`}>{text.reviewChangeLabel}</label>
            <textarea
              id={`${id}-change`}
              value={change}
              disabled={busy}
              rows={3}
              maxLength={REVIEW_CHANGE_NOTE_BYTES}
              onChange={(event) => setChange(event.target.value)}
            />
          </div>
        )}
        {!readOnly && !closed && (
          <p className="review-recipient">
            {recipient ? agentSentence(text.reviewTo, recipient) : text.reviewNoRecipient}
          </p>
        )}
        {outcome?.state === 'working' && <p role="status">{text.reviewSaving}</p>}
        {/* A failure that predates the section closing says nothing about a closed section. */}
        {outcome?.state === 'failed' && outcome.step === 'reopening' && (
          <p role="alert">{text.reviewReopenFailed}</p>
        )}
        {outcome?.state === 'failed' && outcome.step !== 'reopening' && !closed && (
          <p role="alert">
            {outcome.step === 'closing' ? text.reviewCommentKept : text.reviewFailed}
          </p>
        )}
        {outcome?.state === 'uncertain' && !closed && <p role="status">{text.reviewUncertain}</p>}
        {outcome?.state === 'settled' && outcome.warnings && outcome.warnings.length > 0 && (
          <p role="status">
            {text.threadNotNotified} <AgentList names={outcome.warnings} />
          </p>
        )}
        {children}
      </div>
    </section>
  );
}
