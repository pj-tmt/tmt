import { BrowserAction } from '@tmt/browser-ui/react';
import { useSyncExternalStore } from 'react';
import {
  anonymousText,
  messageParts,
  USER_NAME_COMMAND,
  type SendIdentityNotice,
} from './send-identity-notice.js';
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
          const { before, command, after } = messageParts(anonymousText(identity));
          return (
            <>
              {before}
              {command && <code className="tmt-ui-code">{USER_NAME_COMMAND}</code>}
              {after}
            </>
          );
        })()}
      </>
    ) : (
      text.sendRenamed(identity.name, identity.renamedFrom)
    );
  return (
    <p
      role="status"
      className="annotation-hint"
      data-testid="send-identity"
      data-reason={identity.state === 'anonymous' ? identity.reason : undefined}
    >
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
