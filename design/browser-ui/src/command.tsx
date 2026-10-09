import type { ReactNode } from 'react';
import { browserUiClasses as c } from './static.js';

export interface BrowserCommandProps {
  text: string;
  action?: ReactNode;
  feedback?: ReactNode;
}

/** Literal selectable text; the host owns copying, feedback and async lifetime. */
export function BrowserCommand({ text, action, feedback }: BrowserCommandProps) {
  return (
    <div className={c.command}>
      <pre className={c.commandText}>
        <code>{text}</code>
      </pre>
      {action}
      {feedback && (
        <p className={c.commandFeedback} role="status">
          {feedback}
        </p>
      )}
    </div>
  );
}
