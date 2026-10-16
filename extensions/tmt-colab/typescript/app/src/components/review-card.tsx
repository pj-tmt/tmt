import { BrowserAction } from '@tmt/browser-ui/react';
import { Check, CircleDot, Diamond } from 'lucide-react';
import { useEffect, useId, useState, type ReactNode } from 'react';
import type { Review } from '../questions.js';
import type { ReviewOutcome } from '../review-actions.js';
import type { ReviewState } from '../reviews.js';
import { REVIEW_CHANGE_NOTE_BYTES, REVIEW_CONFIRM_NOTE_POINTS } from '../reviews.js';
import { compactRelativeTime } from '../display-time.js';
import { text } from '../strings.js';

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
  const status =
    state.kind === 'closed'
      ? text.reviewClosed
      : state.kind === 'changes'
        ? text.reviewChanges
        : text.reviewWaiting;
  const detail =
    state.kind === 'closed'
      ? [
          state.by ? text.reviewClosedBy(state.by) : undefined,
          now && state.at && Number.isFinite(Number(state.at))
            ? compactRelativeTime(Number(state.at), now)
            : undefined,
        ].filter(Boolean)
      : [];
  return (
    <section
      className="review-card"
      aria-label={text.reviewSection(review.title)}
      data-review-id={review.id}
      data-review-state={state.kind}
    >
      <header>
        <span className="review-state">
          {closed ? (
            <Check aria-hidden />
          ) : state.kind === 'changes' ? (
            <Diamond aria-hidden />
          ) : (
            <CircleDot aria-hidden />
          )}
          <strong>{status}</strong>
          <span className="review-section-title">{review.title}</span>
        </span>
        {detail.length > 0 && <span className="review-detail">{detail.join(' · ')}</span>}
        {!readOnly && closed && (
          <BrowserAction
            type="button"
            variant="text"
            label={text.reviewReopen}
            disabled={busy}
            onActivate={(event) => {
              if (event.isTrusted) reopen();
            }}
          />
        )}
      </header>
      {state.kind !== 'waiting' && state.note && <p className="review-note">{state.note}</p>}
      {!readOnly && !closed && (
        <div className="review-actions">
          {noting && (
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
          {changing && (
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
          <div className="review-buttons">
            {!changing && (
              <>
                <BrowserAction
                  type="button"
                  variant="primary"
                  label={text.reviewConfirm}
                  disabled={busy}
                  onActivate={(event) => {
                    if (event.isTrusted) confirm(noting ? note : '');
                  }}
                />
                {!noting && (
                  <BrowserAction
                    type="button"
                    variant="text"
                    label={text.reviewAddNote}
                    disabled={busy}
                    onActivate={(event) => {
                      if (event.isTrusted) setNoting(true);
                    }}
                  />
                )}
                <BrowserAction
                  type="button"
                  variant="text"
                  label={text.reviewNeedsChange}
                  disabled={busy}
                  onActivate={(event) => {
                    if (!event.isTrusted) return;
                    setNoting(false);
                    setNote('');
                    setChanging(true);
                  }}
                />
              </>
            )}
            {changing && (
              <>
                <BrowserAction
                  type="button"
                  variant="primary"
                  label={text.reviewChangeSend}
                  disabled={busy || !change.trim()}
                  onActivate={(event) => {
                    if (event.isTrusted) requestChange(change);
                  }}
                />
                <BrowserAction
                  type="button"
                  variant="text"
                  label={text.reviewCancel}
                  disabled={working}
                  onActivate={(event) => {
                    if (!event.isTrusted) return;
                    setChanging(false);
                    setChange('');
                  }}
                />
              </>
            )}
          </div>
          <p className="review-recipient">
            {recipient ? text.reviewTo(recipient) : text.reviewNoRecipient}
          </p>
        </div>
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
          {text.threadNotNotified(outcome.warnings.map((name) => `@${name}`).join(', '))}
        </p>
      )}
      {children}
    </section>
  );
}
