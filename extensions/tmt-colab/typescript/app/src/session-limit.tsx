import { BrowserCommand } from '@tmt/browser-ui/react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { text } from './strings.js';

/**
 * The card body for a device at its Remote session limit. Both bodies state the cause first, then
 * the fixes: close another page, the one command that raises the limit and the settings link
 * (#2625). A tab that was disconnected leads with its own sentence; a refused open with the limit.
 */
export function SessionLimitBody({
  limit,
  settingsUrl,
  refused,
}: {
  limit: number;
  settingsUrl?: string;
  /** The open was refused; otherwise the tab was disconnected. */
  refused?: boolean;
}) {
  return (
    <>
      {refused ? (
        <>
          <p>{text.sessionLimitReached(limit)}</p>
          <p>{text.sessionLimitClose}</p>
        </>
      ) : (
        <p>{text.sessionEvicted(limit)}</p>
      )}
      <p>{text.sessionLimitCommand} </p>
      <BrowserCommand text={`tmt remote settings sessions-per-device ${limit + 1}`} />
      {settingsUrl && (
        <p>
          <a className={ui.action} data-variant="text" href={settingsUrl}>
            {text.remoteSettings}
          </a>
        </p>
      )}
    </>
  );
}
