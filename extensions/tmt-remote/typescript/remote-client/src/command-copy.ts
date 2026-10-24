/** A terminal reference with best-effort Copy and inline status; it never executes the command. */
import { browserUiClasses } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';

export function commandCopy(text: string): HTMLElement {
  const command = document.createElement('div');
  command.className = browserUiClasses.command;
  const code = document.createElement('code');
  code.className = browserUiClasses.commandText;
  code.textContent = text;
  const copy = document.createElement('button');
  copy.className = browserUiClasses.commandCopy;
  copy.type = 'button';
  copy.setAttribute('aria-label', `Copy ${text}`);
  copy.append(createIcon('copy'), 'Copy');
  const feedback = document.createElement('span');
  feedback.className = browserUiClasses.commandFeedback;
  feedback.setAttribute('role', 'status');
  copy.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(text);
      feedback.textContent = 'Copied.';
    } catch {
      feedback.textContent = 'Copy failed. Select the command and copy it manually.';
    }
  });
  command.append(code, copy, feedback);
  return command;
}
