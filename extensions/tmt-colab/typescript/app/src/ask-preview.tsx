import { useEffect, useRef, useState } from 'react';
import { AskAttempt, type AskState } from './ask-attempt.js';
import { escapedPreview } from './ask-intent.js';
import { text } from './strings.js';

/** Trusted parent component. The caller supplies an already-admitted, frozen
 * selection; it is not connected to renderer messages, sync or production sends. */
export function AskPreview({ attempt, close }: { attempt: AskAttempt; close(): void }) {
  const [state, setState] = useState<Readonly<AskState>>(attempt.state);
  const current = useRef<AskAttempt | null>(attempt);
  useEffect(() => {
    current.current = attempt;
    setState(attempt.state);
    return () => {
      current.current = null;
    };
  }, [attempt]);
  const view = attempt.preview.view;
  const outcome = {
    preparing: text.askPreparing,
    failed: text.askFailed,
    held: text.askHeld,
    accepted: text.askAccepted,
    uncertain: text.askUncertain,
    refused: text.askRefused,
    cancelled: text.askCancelled,
  };
  async function send() {
    const pending = attempt.send();
    setState(attempt.state);
    const next = await pending;
    if (current.current === attempt) setState(next);
  }
  return (
    <section className="ask-preview" aria-label={text.askPreview}>
      <h2>{text.askPreview}</h2>
      <dl>
        <dt>{text.askMachine}</dt>
        <dd>
          {view.machineName} · {text[view.online]}
          <br />
          <code>{view.machine}</code>
        </dd>
        <dt>{text.askAgent}</dt>
        <dd>
          {view.agentName} · {text[view.delivery ?? 'deliveryUnknown']}
          <br />
          <code>{view.agent}</code>
        </dd>
      </dl>
      <p>{text.askVisible}</p>
      {view.mode === 'hold' && <p>{text.askHold}</p>}
      <pre aria-label={text.askMessage}>{view.message}</pre>
      <details>
        <summary>{text.askEscaped}</summary>
        <pre>{escapedPreview(attempt.preview.finalBytes())}</pre>
      </details>
      <p className="isolation-note">{text.askAdaptation}</p>
      {!attempt.available && <p role="status">{text.askUnavailable}</p>}
      {state.state !== 'preview' && <p role="status">{outcome[state.state]}</p>}
      <div className="ask-actions">
        <button
          disabled={!attempt.available || state.state !== 'preview'}
          onClick={(event) => {
            if (event.isTrusted) void send();
          }}
        >
          {text.askSend}
        </button>
        <button onClick={close}>{text.askClose}</button>
      </div>
    </section>
  );
}
