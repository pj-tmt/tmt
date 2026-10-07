import { Circle, CircleDot, CircleHelp } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { AskBinding } from './ask-panel.js';
import type { AgentDirectoryObservation } from './live-ask.js';
import { text } from './strings.js';
import './agent-status-panel.css';

type AgentStatusBinding = Pick<AskBinding, 'observeDestinations'>;
type SuccessfulRead = Extract<AgentDirectoryObservation, { kind: 'ready' }>;
type StatusView = {
  binding: AgentStatusBinding;
  page: string;
  state: 'loading' | 'done' | 'page-unavailable';
  observation?: AgentDirectoryObservation;
  last?: SuccessfulRead;
};

/** Presentation only. The existing facade owns admitted reads; this view has no
 * session recovery, destination admission, publication or message capability. */
export function AgentStatusPanel({
  open,
  binding,
  page,
  admitted,
}: {
  open: boolean;
  binding?: AgentStatusBinding;
  page: string;
  admitted: boolean;
}) {
  const [view, setView] = useState<StatusView>();
  const pending = useRef<object | undefined>(undefined);
  // Admission loss invalidates the cache even if the same page/client later returns.
  // Reset before rendering so a restored scope cannot expose an old successful read.
  if (!admitted || (view && (view.binding !== binding || view.page !== page))) {
    pending.current = undefined;
    if (view) setView(undefined);
  }
  const read = useCallback(() => {
    if (!admitted || !binding?.observeDestinations || pending.current) return;
    const request = (pending.current = {});
    setView((previous) => ({
      binding,
      page,
      state: 'loading',
      last: previous?.binding === binding && previous.page === page ? previous.last : undefined,
    }));
    void binding.observeDestinations().then(
      (observation) => {
        if (pending.current !== request) return;
        pending.current = undefined;
        setView((previous) => ({
          binding,
          page,
          state: 'done',
          observation,
          last:
            observation.kind === 'ready'
              ? observation
              : observation.phase === 'session' ||
                  observation.failure === 'ended' ||
                  observation.failure === 'evicted' ||
                  observation.code === 'REMOTE_SCOPE_DENIED' ||
                  observation.code === 'REMOTE_SEQUENCE_UNAVAILABLE'
                ? undefined
                : previous?.binding === binding && previous.page === page
                  ? previous.last
                  : undefined,
        }));
      },
      () => {
        if (pending.current !== request) return;
        pending.current = undefined;
        setView({ binding, page, state: 'page-unavailable' });
      },
    );
  }, [admitted, binding, page]);
  useEffect(() => {
    if (open) read();
    return () => {
      pending.current = undefined;
    };
  }, [open, read]);
  // Do not expose a previous page/client snapshot while effects admit the new generation.
  const current =
    admitted && view && view.binding === binding && view.page === page ? view : undefined;
  const observation = current?.observation;
  const last = current?.last;
  const loading = current?.state === 'loading';
  const stale = !!last && observation?.kind !== 'ready';
  const failure = observation?.kind === 'unavailable' ? observation : undefined;
  const channelUnavailable = failure?.code === 'REMOTE_SEQUENCE_UNAVAILABLE';
  const failureLabel = channelUnavailable
    ? text.agentStatusChannelUnavailable
    : failure
      ? failure.failure === 'ended'
        ? text.agentStatusEnded
        : failure.failure === 'evicted'
          ? text.agentStatusEvicted
          : failure.failure === 'refused'
            ? text.agentStatusRefused
            : text.agentStatusUnavailable
      : text.agentStatusUnavailable;
  const session = loading
    ? text.agentStatusChecking
    : observation?.kind === 'ready'
      ? text.agentStatusRead
      : channelUnavailable ||
          failure?.failure === 'ended' ||
          failure?.failure === 'evicted' ||
          failure?.phase === 'session'
        ? failureLabel
        : failure?.phase === 'directory'
          ? text.agentStatusRead
          : text.agentStatusUnavailable;
  const directory = loading
    ? text.agentStatusChecking
    : observation?.kind === 'ready'
      ? text.agentStatusRead
      : failure?.phase === 'directory'
        ? failureLabel
        : text.agentStatusUnavailable;
  return (
    <section className="agent-status-panel" aria-label={text.agentStatus}>
      <dl className="agent-status-health" aria-live="polite">
        <dt>{text.agentStatusPage}</dt>
        <dd>
          {admitted && current?.state !== 'page-unavailable'
            ? text.agentStatusAdmitted
            : text.agentStatusUnavailable}
        </dd>
        <dt>{text.agentStatusSession}</dt>
        <dd>{session}</dd>
        <dt>{text.agentStatusDirectory}</dt>
        <dd>{directory}</dd>
      </dl>
      {failure?.code && (
        <p className="agent-status-code">
          <code>{failure.code}</code>
        </p>
      )}
      <button
        type="button"
        disabled={!admitted || !binding?.observeDestinations || loading}
        onClick={(event) => {
          if (event.isTrusted) read();
        }}
      >
        {text.agentStatusRecheck}
      </button>
      {last && (
        <p className="agent-status-checked">
          {text.agentStatusLastCheck}:{' '}
          <time dateTime={new Date(last.checkedAt).toISOString()}>
            {new Date(last.checkedAt).toLocaleString()}
          </time>
        </p>
      )}
      {stale && (
        <p className="agent-status-stale" role="status">
          {text.agentStatusStale}
        </p>
      )}
      {last ? (
        last.destinations.length ? (
          <ul className="agent-status-list">
            {last.destinations.map((agent, index) => {
              const presence = agent.presence ?? 'unknown';
              const Mark =
                presence === 'active' ? CircleDot : presence === 'offline' ? Circle : CircleHelp;
              const label =
                presence === 'active'
                  ? text.agentStatusActive
                  : presence === 'offline'
                    ? text.presenceOffline
                    : text.presenceUnknown;
              return (
                <li key={`${agent.machine}:${agent.agent}:${index}`} data-presence={presence}>
                  <div>
                    <strong>{agent.agentName}</strong>
                    <span className="agent-status-presence">
                      <Mark aria-hidden />
                      {label}
                    </span>
                  </div>
                  <p>{agent.machineName}</p>
                  <code>{agent.agent}</code>
                </li>
              );
            })}
          </ul>
        ) : (
          <p>{text.agentStatusEmpty}</p>
        )
      ) : (
        <p>{text.agentStatusNoRead}</p>
      )}
      <p className="agent-status-note">{text.agentStatusReadOnly}</p>
    </section>
  );
}
