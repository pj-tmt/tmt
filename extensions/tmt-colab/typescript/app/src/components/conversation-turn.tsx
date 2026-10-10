import { useEffect, useState, type ComponentPropsWithoutRef, type ReactNode } from 'react';
import { compactRelativeTime } from '../display-time.js';
import { text } from '../strings.js';
import { Bot } from 'lucide-react';
import type { RunningDriver } from '../ask-remote.js';

/** Presentation only: callers supply admitted labels, text and trusted actions. */
export function ConversationTurn({
  role,
  runningDriver,
  author,
  at,
  authorTitle,
  bylineTestId,
  meta,
  actions,
  status,
  delivery,
  children,
  className = '',
  ...attributes
}: Omit<ComponentPropsWithoutRef<'article'>, 'role'> & {
  role: 'user' | 'agent';
  runningDriver?: RunningDriver;
  author: string;
  at: number;
  authorTitle?: string;
  bylineTestId?: string;
  meta?: ReactNode;
  actions?: ReactNode;
  status?: ReactNode;
  delivery?: ReactNode;
}) {
  const driver =
    role === 'agent' && (runningDriver === 'claude' || runningDriver === 'codex')
      ? runningDriver
      : undefined;
  const [now, setNow] = useState(0);
  useEffect(() => setNow(Date.now()), [at]);
  return (
    <article
      {...attributes}
      className={`conversation-turn ${className}`}
      data-turn-role={role}
      data-running-driver={driver}
    >
      <div className="comment-main conversation-main">
        <header>
          <p className="comment-byline" title={authorTitle} data-testid={bylineTestId}>
            {driver && (
              <span className="conversation-avatar" aria-hidden="true">
                <Bot size={16} />
              </span>
            )}
            <span className="conversation-author">{author}</span>
            {driver ? (
              <span className="conversation-driver"> {driver}</span>
            ) : (
              role === 'agent' && ` · ${text.conversationAgent}`
            )}{' '}
            ·{' '}
            <time dateTime={new Date(at).toISOString()} title={new Date(at).toISOString()}>
              {compactRelativeTime(at, now)}
            </time>
            {meta}
          </p>
          {actions && <span className="comment-header-end">{actions}</span>}
        </header>
        <div className="conversation-body">{children}</div>
        {status && <div className="conversation-turn-status">{status}</div>}
        {delivery && <div className="conversation-delivery">{delivery}</div>}
      </div>
    </article>
  );
}
