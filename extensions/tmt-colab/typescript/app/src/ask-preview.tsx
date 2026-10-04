import { useEffect, useRef, useState } from 'react';
import type { LedgerState } from './ask-records.js';
import type { FrozenAsk } from './ask-intent.js';

/** UI-facing explicit action. Browser fixture facades and the live controller
 * expose the same view without moving persistence or Remote policy into React. */
export interface PreviewAttempt {
  readonly preview: FrozenAsk;
  readonly available: boolean;
  readonly state: Readonly<{ state: LedgerState | 'preview' | 'preparing' }>;
  send(): Promise<Readonly<{ state: LedgerState | 'preview' | 'preparing' }>>;
}
import { escapedPreview } from './ask-intent.js';
import { text } from './strings.js';

/** Trusted parent component. The caller supplies an already-admitted, frozen
 * selection. The renderer never receives this component or its signing handle. */
export function AskPreview({
  attempt,
  close,
  sent,
  blocked = false,
}: {
  attempt: PreviewAttempt;
  close(): void;
  /** Called once the Send settled as accepted, held or uncertain: the ask is on the page. */
  sent?(operationId: string): void;
  blocked?: boolean;
}) {
  const [state, setState] = useState(attempt.state);
  const current = useRef<PreviewAttempt | null>(attempt);
  useEffect(() => {
    current.current = attempt;
    setState(attempt.state);
    return () => {
      current.current = null;
    };
  }, [attempt]);
  const [expired, setExpired] = useState(false);
  useEffect(() => {
    const expiresAt = Math.min(
      attempt.preview.expiresAt,
      attempt.preview.view.grantExpiresAt ?? Infinity,
    );
    const remaining = expiresAt - Date.now();
    setExpired(remaining <= 0);
    const timer = setTimeout(() => setExpired(true), Math.max(0, remaining));
    return () => clearTimeout(timer);
  }, [attempt]);
  const view = attempt.preview.view;
  const offline = blocked || view.online === 'offline';
  const outcome = {
    preparing: text.askPreparing,
    dispatching: text.askDispatching,
    expired: text.askExpiredState,
    abandoned: text.askAbandoned,
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
    if (current.current !== attempt) return;
    setState(next);
    if (['accepted', 'held', 'uncertain'].includes(next.state)) sent?.(view.operationId);
  }
  return (
    <section
      className="ask-preview"
      aria-label={text.askPreview}
      data-testid="ask-preview"
      data-operation-id={view.operationId}
    >
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
          {view.agentName}
          <br />
          <code>{view.agent}</code>
        </dd>
      </dl>
      <p>{text.askVisible}</p>
      {view.mode === 'hold' && <p>{text.askHold}</p>}
      <pre aria-label={text.askMessage} data-testid="ask-preview-text">
        {view.deliveredMessage}
      </pre>
      <details>
        <summary>{text.askEscaped}</summary>
        <pre>{escapedPreview(attempt.preview.deliveredBytes())}</pre>
      </details>
      {view.message.includes('!') && <p className="isolation-note">{text.askAdaptation}</p>}
      {offline && <p role="status">{text.askOffline}</p>}
      {expired && <p role="status">{text.askExpired}</p>}
      {!attempt.available && <p role="status">{text.askUnavailable}</p>}
      {state.state !== 'preview' && <p role="status">{outcome[state.state]}</p>}
      <div className="ask-actions">
        <button
          data-testid="ask-send"
          disabled={!attempt.available || offline || expired || state.state !== 'preview'}
          onClick={(event) => {
            if (event.isTrusted && !offline && !expired) void send();
          }}
        >
          {text.askSend}
        </button>
        <button onClick={close}>{text.askClose}</button>
      </div>
    </section>
  );
}
