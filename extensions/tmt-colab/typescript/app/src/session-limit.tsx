import { BrowserCommand } from '@tmt/browser-ui/react';
import { text } from './strings.js';

/**
 * The card body for a device at its Remote session limit. A tab that was disconnected by the
 * limit and a page whose open was refused by it read the same way after their first sentences:
 * the limit, the one command that raises it, and the settings link (#2625).
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
          <p>{text.sessionLimitClose}</p>
          <p>{text.sessionLimitReached(limit)}</p>
        </>
      ) : (
        <p>{text.sessionEvicted(limit)}</p>
      )}
      <p>{text.sessionLimitCommand} </p>
      <BrowserCommand text={`tmt remote settings sessions-per-device ${limit + 1}`} />
      {settingsUrl && (
        <p>
          <a href={settingsUrl}>{text.remoteSettings}</a>
        </p>
      )}
    </>
  );
}
