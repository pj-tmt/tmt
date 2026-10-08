import { useEffect, useState, type ComponentPropsWithoutRef, type ReactNode } from 'react';
import { compactRelativeTime } from '../display-time.js';
import { text } from '../strings.js';

/** Presentation only: callers supply admitted labels, text and trusted actions. */
export function ConversationTurn({
  role,
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
  author: string;
  at: number;
  authorTitle?: string;
  bylineTestId?: string;
  meta?: ReactNode;
  actions?: ReactNode;
  status?: ReactNode;
  delivery?: ReactNode;
}) {
  const [now, setNow] = useState(0);
  useEffect(() => setNow(Date.now()), [at]);
  return (
    <article {...attributes} className={`conversation-turn ${className}`} data-turn-role={role}>
      <div className="comment-main conversation-main">
        <header>
          <p className="comment-byline" title={authorTitle} data-testid={bylineTestId}>
            {author}
            {role === 'agent' && ` · ${text.conversationAgent}`} ·{' '}
            <time dateTime={new Date(at).toISOString()} title={new Date(at).toISOString()}>
              {compactRelativeTime(at, now)}
            </time>
            {meta}
          </p>
          {(status || actions) && (
            <span className="comment-header-end">
              {status}
              {actions}
            </span>
          )}
        </header>
        <div className="conversation-body">{children}</div>
        {delivery && <div className="conversation-delivery">{delivery}</div>}
      </div>
    </article>
  );
}
