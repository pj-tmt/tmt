import { useContext } from 'react';
import { AgentChip } from './components/agent-chip.js';
import { ConversationPresentation } from './components/conversation-presentation.js';
import { AskAgainAction } from './ask-again.js';
import type { AskBinding } from './ask-panel.js';
import type { MessageSendFailure } from './comment-send.js';
import { text } from './strings.js';

/** Retained pre-adoption/uncertain outcomes use the same explicit retry as annotations. */
export function CommentSendFailures({
  failures,
  binding,
  blocked,
  settled,
}: {
  failures: readonly MessageSendFailure[];
  binding?: AskBinding;
  blocked: boolean;
  settled(failure: MessageSendFailure, result: { adopted?: boolean }): void;
}) {
  const chat = useContext(ConversationPresentation) === 'chat';
  return (
    <>
      {failures.map((failure, index) => (
        <div
          role="status"
          data-testid="recipient-failure"
          data-message-id={failure.message}
          key={index}
        >
          <p>
            {chat && !failure.uncertain && <span aria-hidden="true">✗ </span>}
            <AgentChip name={failure.agent} /> ·{' '}
            {failure.uncertain ? text.askUnconfirmed : text.askNotDelivered}{' '}
            {!failure.uncertain && (
              <AskAgainAction
                binding={binding}
                blocked={blocked}
                input={failure.input}
                recipient={failure.recipient}
                retryOf={null}
                settled={(outcome) => settled(failure, outcome)}
              />
            )}
          </p>
          {failure.uncertain && <p className="ask-supporting">{text.askUncertain}</p>}
        </div>
      ))}
    </>
  );
}
