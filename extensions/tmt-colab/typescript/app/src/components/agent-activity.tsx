import { createContext, useContext, useEffect, useState, type ReactNode } from 'react';
import { activityAt, type ActivityObservation } from '../agent-activity.js';
import type { PageAsk } from '../ask-panel.js';
import { text } from '../strings.js';

const ActivityPresentation = createContext<{ observation?: ActivityObservation; now: number }>({
  now: 0,
});

/** One page clock paints elapsed time; network reads belong only to the Ask observer. */
export function AgentActivityProvider({
  observation,
  children,
}: {
  observation?: ActivityObservation;
  children: ReactNode;
}) {
  const [now, setNow] = useState(0);
  const [visible, setVisible] = useState(true);
  useEffect(() => {
    const visibility = () => setVisible(document.visibilityState !== 'hidden');
    visibility();
    document.addEventListener('visibilitychange', visibility);
    return () => document.removeEventListener('visibilitychange', visibility);
  }, []);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const update = () => {
      const timestamp = performance.now();
      setNow(timestamp);
      if (
        visible &&
        observation?.active &&
        timestamp - observation.receivedAt < 2 * observation.intervalMs
      )
        timer = setTimeout(update, 1000);
    };
    update();
    return () => clearTimeout(timer);
  }, [observation, visible]);
  const current = visible ? observation : observation && { ...observation, active: false };
  return (
    <ActivityPresentation value={{ observation: current, now }}>{children}</ActivityPresentation>
  );
}

function duration(ms: number, seconds: boolean): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return seconds
    ? `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
    : `${Math.floor(total / 60)}m`;
}

/** Delivery outcomes stay in AskPanel; this line never changes the signed ledger. */
export function AgentActivityLine({ record }: { record: PageAsk }) {
  const { observation, now } = useContext(ActivityPresentation);
  const activity = activityAt(observation, record, now);
  const name = record.agentName || text.askAgentLabel;
  const kind = activity?.kind ?? 'sent';
  const elapsed = activity?.elapsedMs ?? 0;
  const label =
    kind === 'working'
      ? text.agentWorking(name, duration(elapsed, true))
      : kind === 'queued'
        ? text.agentQueued(name, duration(elapsed, false))
        : kind === 'finished'
          ? text.agentFinished(name, duration(elapsed, false))
          : kind === 'unknown'
            ? text.agentActivityUnknown(name)
            : text.agentSent;
  return (
    <span className="agent-activity" data-activity={kind}>
      {kind === 'working' && (
        <span className="agent-activity-spinner" aria-hidden>
          ⠹
        </span>
      )}
      {kind === 'queued' && <span aria-hidden>◌</span>}
      {(kind === 'finished' || kind === 'unknown') && <span aria-hidden>✓</span>}
      {label}
    </span>
  );
}
