import { BrowserAction } from '@tmt/browser-ui/react';
import { useSyncExternalStore } from 'react';
import { messageParts, type SendIdentityNotice } from './send-identity-notice.js';
import { text } from './strings.js';

/** A quiet line under the composer about how the last send was attributed; informational only. */
export function SendIdentityLine({ notice }: { notice: SendIdentityNotice }) {
  const identity = useSyncExternalStore(notice.subscribe, notice.current);
  if (!identity) return null;
  const body =
    identity.state === 'anonymous' ? (
      <>
        {text.sendAnonymous}{' '}
        {(() => {
          const { before, command, after } = messageParts(identity.message);
          return (
            <>
              {before}
              {command && <code className="tmt-ui-code">{'tmt config set user.name <name>'}</code>}
              {after}
            </>
          );
        })()}
      </>
    ) : (
      text.sendRenamed(identity.name, identity.renamedFrom)
    );
  return (
    <p role="status" className="annotation-hint" data-testid="send-identity">
      {body}{' '}
      <BrowserAction
        type="button"
        variant="text"
        label={text.sendIdentityDismiss}
        onActivate={(event) => {
          if (event.isTrusted) notice.dismiss();
        }}
      />
    </p>
  );
}
