import { ConversationPresentation } from './conversation-presentation.js';
import {
  useContext,
  useEffect,
  useState,
  type ComponentPropsWithoutRef,
  type ReactNode,
} from 'react';
import { BrowserAvatar } from '@tmt/browser-ui/react';
import { compactRelativeTime } from '../display-time.js';
import { AgentChip } from './agent-chip.js';

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
  const chat = useContext(ConversationPresentation) === 'chat';
  const [now, setNow] = useState(0);
  useEffect(() => setNow(Date.now()), [at]);
  return (
    <article {...attributes} className={`conversation-turn ${className}`} data-turn-role={role}>
      <div className="comment-main conversation-main">
        <header>
          {!chat && <BrowserAvatar name={author} size="s" decorative />}
          <p className="comment-byline" title={authorTitle} data-testid={bylineTestId}>
            <span className="conversation-author">
              {role === 'agent' ? <AgentChip name={author} mark={chat} /> : author}
            </span>{' '}
            <span className="conversation-time-separator">· </span>
            <time dateTime={new Date(at).toISOString()} title={new Date(at).toISOString()}>
              {compactRelativeTime(at, now)}
            </time>
            {meta}
          </p>
          {(actions || chat) && <span className="comment-header-end">{actions}</span>}
        </header>
        <div className="conversation-body">{children}</div>
        {status && <div className="conversation-turn-status">{status}</div>}
        {delivery && <div className="conversation-delivery">{delivery}</div>}
      </div>
    </article>
  );
}
