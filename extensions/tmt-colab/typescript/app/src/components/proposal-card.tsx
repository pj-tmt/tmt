import { BrowserAction } from '@tmt/browser-ui/react';
import { Check, CircleCheck, CircleDot, X } from 'lucide-react';
import { useId, useLayoutEffect, useRef, type ReactNode } from 'react';
import type { ThreadView } from '../thread-records.js';
import type { ProposalOutcome } from '../proposal-actions.js';
import { text } from '../strings.js';

/** Authenticated parent presentation only; callbacks own all admission/effects. */
export function ProposalCard({
  thread,
  outcome,
  disabled,
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
}: {
  thread: ThreadView;
  outcome?: ProposalOutcome;
  disabled: boolean;
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
}) {
  const proposal = thread.proposal!;
  const titleId = useId();
  const card = useRef<HTMLElement>(null);
  const followUpControl = useRef<HTMLSpanElement>(null);
  const focused = useRef<HTMLElement | null>(null);
  const returning = useRef(false);
  const busy = outcome?.state === 'deciding' || outcome?.state === 'notifying';
  const decision = thread.decision?.decision;
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
    if (next && !next.disabled) {
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
        {thread.resolved && resolve && (
          <BrowserAction
            type="button"
            variant="text"
            label={text.threadReopen}
            disabled={disabled || busy}
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
      {!thread.resolved && (
        <div className="proposal-actions">
          {!decision && !outcome && !thread.resolved && (
            <>
              <span className="proposal-approve">
                <BrowserAction
                  type="button"
                  variant="primary"
                  label={text.proposalApprove}
                  disabled={disabled || !canDecide}
                  onActivate={(e) => {
                    if (e.isTrusted) decide('approved');
                  }}
                />
              </span>
              <BrowserAction
                type="button"
                variant="primary"
                label={text.proposalDecline}
                disabled={disabled || !canDecide}
                onActivate={(e) => {
                  if (e.isTrusted) decide('declined');
                }}
              />
            </>
          )}
          <span ref={followUpControl}>
            <BrowserAction
              type="button"
              variant="text"
              label={text.proposalFollowUp}
              disabled={disabled || busy}
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
                disabled={disabled || busy}
                onActivate={(e) => {
                  if (e.isTrusted) resolve();
                }}
              />
            </span>
          )}
        </div>
      )}
      {statusError && <p role="alert">{text.commentFailed}</p>}
      {outcome?.state === 'deciding' && <p role="status">{text.proposalSaving}</p>}
      {outcome?.state === 'notifying' && <p role="status">{text.askPreparing}</p>}
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
