import { useState } from 'react';
import { BrowserCommand } from '@tmt/browser-ui/react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { text } from './strings.js';

/** Clipboard feedback says only what copying did, never that a machine step ran. */
export function CopyCommand({ command, enabled }: { command: string; enabled: boolean }) {
  const [feedback, setFeedback] = useState('');
  return (
    <BrowserCommand
      text={command}
      action={
        <button
          type="button"
          className={ui.commandCopy}
          disabled={!enabled}
          onClick={async (event) => {
            if (!event.isTrusted) return;
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
      }
      feedback={feedback}
    />
  );
}
