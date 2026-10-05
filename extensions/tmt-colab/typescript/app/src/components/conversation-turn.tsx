import { Bot, User } from 'lucide-react';
import { useEffect, useState, type ComponentPropsWithoutRef, type ReactNode } from 'react';
import { compactRelativeTime } from '../display-time.js';
import { text } from '../strings.js';

/** Presentation only: callers supply admitted labels, text and trusted actions. */
export function ConversationTurn({
  role,
  layout,
  author,
  at,
  authorTitle,
  bylineTestId,
  meta,
  actions,
  delivery,
  children,
  className = '',
  ...attributes
}: Omit<ComponentPropsWithoutRef<'article'>, 'role'> & {
  role: 'user' | 'agent';
  layout: 'chat' | 'thread';
  author: string;
  at: number;
  authorTitle?: string;
  bylineTestId?: string;
  meta?: ReactNode;
  actions?: ReactNode;
  delivery?: ReactNode;
}) {
  const [now, setNow] = useState(0);
  useEffect(() => setNow(Date.now()), [at]);
  return (
    <article
      {...attributes}
      className={`conversation-turn chat-${role}-turn ${className}`}
      data-turn-role={role}
      data-turn-layout={layout}
    >
      {layout === 'thread' && (
        <span className="conversation-avatar" aria-hidden>
          {role === 'agent' ? <Bot /> : <User />}
        </span>
      )}
      <div className="comment-main conversation-main">
        <header>
          <p className="comment-byline" title={authorTitle} data-testid={bylineTestId}>
            {layout === 'chat' && role === 'agent' && (
              <Bot className="conversation-meta-mark" aria-hidden />
            )}
            {author}
            {role === 'agent' && ` · ${text.conversationAgent}`} ·{' '}
            <time dateTime={new Date(at).toISOString()} title={new Date(at).toISOString()}>
              {compactRelativeTime(at, now)}
            </time>
            {meta}
          </p>
          {actions && <span className="comment-header-end">{actions}</span>}
        </header>
        <div className="conversation-body">{children}</div>
        {delivery && <div className="conversation-delivery">{delivery}</div>}
      </div>
    </article>
  );
}
