import { useState } from 'react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { text } from './strings.js';
import './shell-notice.css';

/** Presentation only: the command never opens a terminal or runs on this computer. */
export function ShellNotice({ pageId, canCopy = false }: { pageId?: string; canCopy?: boolean }) {
  const [feedback, setFeedback] = useState('');
  const command = pageId ? `tmt colab shell read ${pageId}` : undefined;
  return (
    <section className="shell-notice">
      <p role="status">{text.shellPageNotice}</p>
      {command && (
        <div className={ui.command}>
          <code className={ui.commandText}>{command}</code>
          {canCopy && (
            <button
              className={ui.commandCopy}
              type="button"
              onClick={async () => {
                try {
                  await navigator.clipboard.writeText(command);
                  setFeedback(text.shellCommandCopied);
                } catch {
                  setFeedback(text.shellCommandCopyFailed);
                }
              }}
            >
              Copy
            </button>
          )}
          {feedback && (
            <p className={ui.commandFeedback} role="status">
              {feedback}
            </p>
          )}
        </div>
      )}
    </section>
  );
}
