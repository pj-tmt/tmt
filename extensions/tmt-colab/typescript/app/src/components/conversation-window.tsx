import type { ComponentPropsWithoutRef, ReactNode, Ref } from 'react';

/** Presentation only; callers retain history, placement, draft and action lifetimes. */
export function ConversationWindow({
  title,
  caption,
  actions,
  historyRef,
  historyClassName = '',
  notice,
  composer,
  children,
  className = '',
  ...attributes
}: Omit<ComponentPropsWithoutRef<'section'>, 'title'> & {
  title: ReactNode;
  caption?: string;
  actions: ReactNode;
  historyRef: Ref<HTMLDivElement>;
  historyClassName?: string;
  notice?: ReactNode;
  composer?: ReactNode;
}) {
  return (
    <section {...attributes} className={`conversation-window ${className}`}>
      <header className="conversation-window-bar thread-bar">
        <div className="conversation-heading">
          <h2 className="conversation-title">{title}</h2>
          {caption && <span className="conversation-caption">{caption}</span>}
        </div>
        <span className="thread-bar-actions">{actions}</span>
      </header>
      {notice}
      <div className={`conversation-messages ${historyClassName}`} ref={historyRef}>
        {children}
      </div>
      {composer && <div className="conversation-composer">{composer}</div>}
    </section>
  );
}
