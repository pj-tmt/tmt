import { BrowserIconAction } from '@tmt/browser-ui/react';
import { Send } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import type { AskBinding } from './ask-panel.js';
import type { CommentContext } from './thread-store.js';
import type { PreviewAttempt } from './ask-preview.js';
import { text } from './strings.js';

export type AskAgainInput = Pick<
  Parameters<AskBinding['prepare']>[0],
  'quote' | 'comment' | 'title' | 'url'
> & { context: CommentContext };

/** One explicit parent action, shared by local failures and signed refusals.
 * It prepares a new operation for the original UUID pair, never a comment. */
export function AskAgainAction({
  binding,
  blocked,
  input,
  recipient,
  retryOf,
  settled,
}: {
  binding?: AskBinding;
  blocked: boolean;
  input: AskAgainInput;
  recipient: { machine: string; agent: string };
  retryOf: string | null;
  settled?(outcome: Awaited<ReturnType<PreviewAttempt['send']>>): void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const sending = useRef(false);
  const owner = useRef(binding);
  owner.current = binding;
  const disabled = useRef(blocked);
  disabled.current = blocked;
  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  useEffect(() => {
    sending.current = false;
    setBusy(false);
    setError(false);
  }, [binding]);
  async function send() {
    if (!binding || blocked || sending.current || uncertain) return;
    sending.current = true;
    setBusy(true);
    setError(false);
    const captured = structuredClone({ input, recipient, retryOf });
    const current = () => active.current && owner.current === binding && !disabled.current;
    let keepBusy = false;
    try {
      const destinations = await binding.destinations();
      if (!current()) return;
      const matches = destinations.filter(
        (value) =>
          value.machine === captured.recipient.machine && value.agent === captured.recipient.agent,
      );
      if (matches.length !== 1) throw new Error('Recipient unavailable');
      const attempt = await binding.prepare({
        ...captured.input,
        destination: matches[0],
        retryOf: captured.retryOf,
      });
      if (!current()) return;
      // A thrown Send, unlike rejected preparation, may have started adoption.
      let outcome: Awaited<ReturnType<typeof attempt.send>>;
      try {
        outcome = await attempt.send();
      } catch {
        outcome = { state: 'uncertain' };
      }
      if (!active.current || owner.current !== binding) return;
      settled?.(outcome);
      if (outcome.adopted !== false) {
        keepBusy = true;
        if (outcome.adopted !== true) setUncertain(true);
        // Keep the fence until the admitted new record replaces this action.
        return;
      }
    } catch {
      if (current()) setError(true);
    } finally {
      if (active.current && owner.current === binding) {
        setBusy(keepBusy);
        if (!keepBusy) sending.current = false;
      }
    }
  }
  return (
    <span className="conversation-status ask-again-action">
      {uncertain ? (
        <span role="status">{text.askUnconfirmed}</span>
      ) : (
        <BrowserIconAction
          type="button"
          variant="text"
          label={text.askAgain}
          icon={<Send />}
          busy={busy}
          disabled={!binding || blocked}
          onActivate={(event) => {
            if (event.isTrusted) void send();
          }}
        />
      )}
      {error && <span role="alert">{text.askActionFailed}</span>}
    </span>
  );
}
