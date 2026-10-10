import { BrowserAction } from '@tmt/browser-ui/react';
import { Check, CircleDot, X } from 'lucide-react';
import type { ReactNode } from 'react';
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
}) {
  const proposal = thread.proposal!;
  const busy = outcome?.state === 'deciding' || outcome?.state === 'notifying';
  const decision = thread.decision?.decision;
  return (
    <article
      className="proposal-card"
      data-proposal-id={proposal.proposalId}
      data-resolved={thread.resolved || undefined}
    >
      <header>
        <strong>{proposal.title}</strong>
        <span className="proposal-state" data-state={decision ?? 'open'}>
          {decision === 'approved' ? (
            <Check aria-hidden />
          ) : decision === 'declined' ? (
            <X aria-hidden />
          ) : (
            <CircleDot aria-hidden />
          )}
          {thread.resolved
            ? `${decision === 'approved' ? text.proposalApproved + ' · ' : decision === 'declined' ? text.proposalDeclined + ' · ' : ''}${text.threadResolved}`
            : decision === 'approved'
              ? text.proposalApproved
              : decision === 'declined'
                ? text.proposalDeclined
                : text.proposalOpen}
        </span>
      </header>
      <p className="proposal-author">
        {proposal.proposer.label}
        {detached ? ` · ${text.commentDetached}` : ''}
      </p>
      {!thread.resolved && <p className="proposal-body">{proposal.body}</p>}
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
        <BrowserAction
          type="button"
          variant="text"
          label={text.proposalFollowUp}
          disabled={disabled || busy}
          onActivate={(e) => {
            if (e.isTrusted) followUp();
          }}
        />
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
      {statusError && <p role="alert">{text.commentFailed}</p>}
      {outcome?.state === 'deciding' && <p role="status">{text.proposalSaving}</p>}
      {outcome?.state === 'notifying' && <p role="status">{text.askPreparing}</p>}
      {outcome?.state === 'failed' && <p role="alert">{text.proposalNotifyFailed}</p>}
      {outcome?.state === 'uncertain' && (
        <p role="status">
          {outcome.committed ? text.askUnconfirmed : text.proposalDecisionUnconfirmed}
        </p>
      )}
      {!thread.resolved && children}
      {composer}
    </article>
  );
}
