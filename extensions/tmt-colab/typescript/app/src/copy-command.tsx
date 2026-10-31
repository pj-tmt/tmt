import { Copy } from 'lucide-react';
import { useState } from 'react';
import { BrowserCommand } from '@tmt/browser-ui/react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { text } from './strings.js';
import './copy-command.css';

/** Clipboard feedback says only what copying did, never that a machine step ran. */
export function CopyCommand({
  command,
  enabled,
  copyLabel = 'Copy',
  copiedText = text.shellCommandCopied,
  failedText = text.shellCommandCopyFailed,
}: {
  command: string;
  enabled: boolean;
  copyLabel?: string;
  copiedText?: string;
  failedText?: string;
}) {
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
              setFeedback(copiedText);
            } catch {
              setFeedback(failedText);
            }
          }}
        >
          <span className="copy-command-label">
            <Copy size={14} aria-hidden="true" />
            {copyLabel}
          </span>
        </button>
      }
      feedback={feedback}
    />
  );
}
