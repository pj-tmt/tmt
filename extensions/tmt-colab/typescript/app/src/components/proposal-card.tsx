import { BrowserAction, BusyMark, useBusyMark } from '@tmt/browser-ui/react';
import { Check, CircleCheck, CircleDot, X } from 'lucide-react';
import { useId, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import type { ThreadView } from '../thread-records.js';
import type { ProposalOutcome } from '../proposal-actions.js';
import { useNotificationBlock } from '../use-notification-block.js';
import { proposalNotificationDestination } from '../reply-notification-context.js';
import { text } from '../strings.js';

/** Authenticated parent presentation only; callbacks own all admission/effects. */
export function ProposalCard({
  thread,
  outcome,
  disabled,
  sending = false,
  readOnly = false,
  canDecide,
  statusError,
  detached,
  decide,
  followUp,
  resolve,
  children,
  composer,
  historyOpen = false,
  toggleHistory,
  resolvedLabel = text.threadResolved,
  unseen,
  notificationSurface = 'page',
}: {
  thread: ThreadView;
  outcome?: ProposalOutcome;
  disabled: boolean;
  /** A reply composer on this card is sending: its actions are fenced, not greyed. */
  sending?: boolean;
  readOnly?: boolean;
  canDecide: boolean;
  statusError?: boolean;
  detached?: boolean;
  decide(decision: 'approved' | 'declined'): void;
  followUp(): void;
  resolve?(): void;
  children?: ReactNode;
  composer?: ReactNode;
  historyOpen?: boolean;
  toggleHistory?(): void;
  resolvedLabel?: string;
  unseen?: boolean;
  notificationSurface?: 'page' | 'comments';
}) {
  const proposal = thread.proposal!;
  const titleId = useId();
  const card = useRef<HTMLElement>(null);
  useNotificationBlock(
    proposalNotificationDestination(thread.ref.writer, thread.ref.id),
    card,
    !thread.resolved || historyOpen,
    notificationSurface,
  );
  const followUpControl = useRef<HTMLSpanElement>(null);
  const focused = useRef<HTMLElement | null>(null);
  const returning = useRef(false);
  const busy = sending || outcome?.state === 'deciding' || outcome?.state === 'notifying';
  const deciding = outcome?.state === 'deciding';
  const busyOutcome = deciding || outcome?.state === 'notifying';
  // The decision being saved: only its button carries the busy mark, though both stay inert.
  const [pressed, setPressed] = useState<'approved' | 'declined'>();
  const decision = thread.decision?.decision;
  const deciders = !decision && (!outcome || deciding);
  // The row's height while the decision buttons stand in it (they may wrap), with the turns then
  // present. Once the buttons are gone the row keeps that height until the decision's reply turn
  // lands, so the decided state replaces them at the same height and the page then moves once.
  const actions = useRef<HTMLDivElement>(null);
  const [held, setHeld] = useState<{ height: number; turns: number }>();
  const turns = thread.comments.length;
  useLayoutEffect(() => {
    const node = actions.current;
    if (!deciders || !node) return;
    const measure = () => setHeld({ height: node.offsetHeight, turns });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [deciders, turns]);
  const holding = !deciders && !!held && busyOutcome && turns <= held.turns;
  // Said, not shown: the delayed mark has no words, so a screen reader hears the save instead.
  const saving = deciding && !decision;
  const markShown = useBusyMark(saving);
  useLayoutEffect(() => {
    const node = card.current;
    if (!node) return;
    if (
      focused.current &&
      !focused.current.isConnected &&
      document.activeElement === document.body
    ) {
      returning.current = true;
      node.focus({ preventScroll: true });
    }
    if (focused.current && !focused.current.isConnected) focused.current = null;
    if (!returning.current) return;
    if (document.activeElement !== node) {
      returning.current = false;
      return;
    }
    const next = followUpControl.current?.querySelector('button');
    // Focus waits for the control to be usable again: not disabled and not busy.
    if (next && !next.disabled && !next.hasAttribute('aria-busy')) {
      returning.current = false;
      next.focus({ preventScroll: true });
    }
  });
  return (
    <article
      className="proposal-card"
      ref={card}
      tabIndex={-1}
      aria-labelledby={titleId}
      onFocusCapture={(event) => {
        if (event.target !== card.current) focused.current = event.target as HTMLElement;
      }}
      data-proposal-id={proposal.proposalId}
      data-resolved={thread.resolved || undefined}
    >
      <header>
        {thread.resolved && toggleHistory ? (
          <button
            id={titleId}
            className="tmt-ui-action proposal-title"
            type="button"
            data-variant="text"
            title={proposal.title}
            aria-expanded={historyOpen}
            onClick={(event) => {
              if (event.isTrusted) toggleHistory();
            }}
          >
            {proposal.title}
          </button>
        ) : (
          <strong id={titleId} title={proposal.title}>
            {proposal.title}
          </strong>
        )}
        <span
          className="proposal-state"
          data-state={decision ?? 'open'}
          data-unseen={unseen || undefined}
          title={thread.resolved ? resolvedLabel : undefined}
        >
          {thread.resolved ? (
            <CircleCheck aria-hidden />
          ) : decision === 'approved' ? (
            <Check aria-hidden />
          ) : decision === 'declined' ? (
            <X aria-hidden />
          ) : (
            <CircleDot aria-hidden />
          )}
          {thread.resolved
            ? `${decision === 'approved' ? text.proposalApproved + ' · ' : decision === 'declined' ? text.proposalDeclined + ' · ' : ''}${resolvedLabel}`
            : decision === 'approved'
              ? text.proposalApproved
              : decision === 'declined'
                ? text.proposalDeclined
                : text.proposalOpen}
          {thread.resolved && unseen ? ` · ${text.threadUnseen}` : ''}
        </span>
        {!readOnly && thread.resolved && resolve && (
          <BrowserAction
            type="button"
            variant="text"
            label={text.threadReopen}
            busy={busy}
            disabled={disabled}
            onActivate={(event) => {
              if (event.isTrusted) resolve();
            }}
          />
        )}
      </header>
      {(!thread.resolved || historyOpen) && (
        <p className="proposal-author">
          {proposal.proposer.label}
          {detached ? ` · ${text.commentDetached}` : ''}
        </p>
      )}
      {(!thread.resolved || historyOpen) && <p className="proposal-body">{proposal.body}</p>}
      {!readOnly && !thread.resolved && (
        <div
          className="proposal-actions"
          ref={actions}
          style={holding ? { minBlockSize: held.height } : undefined}
        >
          {deciders && (
            // While the decision saves the buttons stay where they are, inert, and the busy mark
            // is the only sign: the row moves nothing until the decision stands in its place.
            <>
              <BrowserAction
                type="button"
                variant="primary"
                label={text.proposalApprove}
                disabled={disabled || !canDecide}
                busy={deciding}
                busyMark={
                  <BusyMark className={pressed === 'approved' ? undefined : 'proposal-unpressed'} />
                }
                onActivate={(e) => {
                  if (!e.isTrusted) return;
                  setPressed('approved');
                  decide('approved');
                }}
              />
              <BrowserAction
                type="button"
                variant="secondary"
                label={text.proposalDecline}
                disabled={disabled || !canDecide}
                busy={deciding}
                busyMark={
                  <BusyMark className={pressed === 'declined' ? undefined : 'proposal-unpressed'} />
                }
                onActivate={(e) => {
                  if (!e.isTrusted) return;
                  setPressed('declined');
                  decide('declined');
                }}
              />
            </>
          )}
          <span ref={followUpControl}>
            <BrowserAction
              type="button"
              variant="text"
              label={text.proposalFollowUp}
              busy={busy}
              disabled={disabled}
              onActivate={(e) => {
                if (e.isTrusted) followUp();
              }}
            />
          </span>
          {resolve && (
            <span className="proposal-resolve">
              <BrowserAction
                type="button"
                variant="text"
                label={thread.resolved ? text.threadReopen : text.threadResolve}
                busy={busy}
                disabled={disabled}
                onActivate={(e) => {
                  if (e.isTrusted) resolve();
                }}
              />
            </span>
          )}
        </div>
      )}
      {/* A live line heard, not seen: it takes no room in the card. */}
      {!readOnly && !thread.resolved && (
        <span className="proposal-announce tmt-ui-visually-hidden" role="status">
          {saving && markShown ? text.proposalSaving : ''}
        </span>
      )}
      {statusError && <p role="alert">{text.commentFailed}</p>}
      {outcome?.state === 'failed' && <p role="alert">{text.proposalNotifyFailed}</p>}
      {outcome?.state === 'uncertain' && (
        <p role="status">
          {outcome.committed ? text.askUnconfirmed : text.proposalDecisionUnconfirmed}
        </p>
      )}
      {(!thread.resolved || historyOpen) && children}
      {!thread.resolved && composer}
    </article>
  );
}
