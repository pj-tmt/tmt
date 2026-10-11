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
  chat = false,
}: {
  chat?: boolean;
  failures: readonly MessageSendFailure[];
  binding?: AskBinding;
  blocked: boolean;
  settled(failure: MessageSendFailure, result: { adopted?: boolean }): void;
}) {
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
            {chat && !failure.uncertain && <span aria-hidden="true">✗ </span>}@{failure.agent} ·{' '}
            {failure.uncertain ? text.askUnconfirmed : text.askNotDelivered}{' '}
            {!failure.uncertain && (
              <AskAgainAction
                label={chat ? text.chatRetry : undefined}
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
