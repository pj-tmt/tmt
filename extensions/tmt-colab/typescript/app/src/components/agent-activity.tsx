import { BusyMark } from '@tmt/browser-ui/react';
import { CircleDashed } from 'lucide-react';
import { createContext, useContext, useEffect, useState, type ReactNode } from 'react';
import { activityAt, type ActivityObservation } from '../agent-activity.js';
import type { PageAsk } from '../ask-panel.js';
import {
  clock,
  remainingAt,
  sentenceCase,
  type MessageStateName,
  type MessageStates,
  type ObservedMessageState,
} from '../message-state.js';
import { text } from '../strings.js';
import { AgentChip, AgentChipActivity } from './agent-chip.js';

const ActivityPresentation = createContext<{
  observation?: ActivityObservation;
  messages?: MessageStates;
  now: number;
}>({
  now: 0,
});

/** Core's state of each sent message, as the observer last read it; none outside the provider. */
export function useMessageStates(): MessageStates | undefined {
  return useContext(ActivityPresentation).messages;
}

/** One page clock paints elapsed time and the countdowns; network reads belong only to the Ask
 * observer. `messages` is Core's state of each sent message, as the observer last read it. */
export function AgentActivityProvider({
  observation,
  messages,
  children,
}: {
  observation?: ActivityObservation;
  messages?: MessageStates;
  children?: ReactNode;
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
      const counting = Object.values(messages ?? {}).some(
        (state) => (remainingAt(state, timestamp) ?? 0) > 0,
      );
      if (
        visible &&
        (counting ||
          (observation?.active && timestamp - observation.receivedAt < 2 * observation.intervalMs))
      )
        timer = setTimeout(update, 1000);
    };
    update();
    return () => clearTimeout(timer);
  }, [observation, messages, visible]);
  const current = visible ? observation : observation && { ...observation, active: false };
  const agents =
    current?.active && now - current.receivedAt < 2 * current.intervalMs
      ? (current.agents ?? [])
      : [];
  return (
    <ActivityPresentation value={{ observation: current, messages, now }}>
      <AgentChipActivity value={agents}>{children}</AgentChipActivity>
    </ActivityPresentation>
  );
}

function duration(ms: number, seconds: boolean): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return seconds
    ? `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
    : `${Math.floor(total / 60)}m`;
}

/** One symbol per state, the same as the Ops TUI; `working` is the line's spinner instead. */
const MARKS: Record<MessageStateName, ReactNode> = {
  sent: '○',
  batched: <DashedCircle />,
  delivered: '✓',
  working: '',
  unconfirmed: '?',
  not_sent: '✗',
};

/** ◌ is missing from most text fonts and falls back to a tiny, faint glyph; the icon set draws it
 * at the size and weight of ○. */
function DashedCircle() {
  return <CircleDashed className="agent-activity-dashed" />;
}

/** Core's wording of the message, unless a Send now is under way or came to nothing (#4136): that
 * replaces it while the message is in the state the press was made in. Core's own sentence for a
 * refusal is drawn as it is; Colab writes one only where Core has none (a device that did not send
 * the message, a failure on Remote's side) and the one general line for a code it does not know. */
function stateWording(shared: ObservedMessageState): string {
  const progress = shared.sendNow;
  if (progress?.phase === 'sending') return text.sendNowSending;
  if (progress?.phase === 'outcome') {
    if (progress.reasonText) return progress.reasonText;
    if (progress.outcome === 'unconfirmed') return text.sendNowUnconfirmed;
    if (progress.outcome === 'unavailable') return text.sendNowUnavailable;
    return progress.reason === 'REMOTE_SCOPE_DENIED'
      ? text.sendNowScopeDenied
      : text.sendNowRefusedUnknown;
  }
  return sentenceCase(shared.reason.text);
}

/** Delivery outcomes stay in AskPanel; this line never changes the signed ledger. When Core has
 * given the state of the message (#4317) the line draws it: Core's wording, one symbol, and the
 * countdown of a batched message. Without it the line is the agent's own activity. */
export function AgentActivityLine({ record }: { record: PageAsk }) {
  const { observation, messages, now } = useContext(ActivityPresentation);
  const activity = activityAt(observation, record, now);
  const name = record.agentName || text.askAgentLabel;
  const shared = messages?.[record.operationId];
  if (shared) {
    const progress = shared.sendNow;
    const sending = progress?.phase === 'sending';
    const unconfirmed = progress?.phase === 'outcome' && progress.outcome === 'unconfirmed';
    const remaining =
      shared.state === 'batched' && !sending && !unconfirmed ? remainingAt(shared, now) : null;
    return (
      <span className="agent-activity" data-message-state={shared.state}>
        {sending || shared.state === 'working' ? (
          <BusyMark className="agent-activity-spinner" />
        ) : (
          <span className="agent-activity-mark" aria-hidden>
            {unconfirmed ? MARKS.unconfirmed : MARKS[shared.state]}{' '}
          </span>
        )}
        {stateWording(shared)}
        {remaining !== null && (
          // The countdown moves to the next line whole, never split inside "in 2:30", and is not
          // read out: only a change of the state is announced, not every second of it.
          <span className="agent-activity-clock" aria-hidden="true">
            {' '}
            · {text.messageStateIn(clock(remaining))}
          </span>
        )}
        {shared.state === 'working' && !sending && activity?.kind === 'working' && (
          <span className="agent-activity-elapsed" aria-hidden="true">
            {' '}
            · {duration(activity.elapsedMs, true)}
          </span>
        )}
      </span>
    );
  }
  const kind = activity?.kind ?? 'sent';
  const elapsed = activity?.elapsedMs ?? 0;
  const detail =
    kind === 'working'
      ? text.agentActivityWorking(duration(elapsed, true))
      : kind === 'queued'
        ? text.agentQueued(duration(elapsed, false))
        : kind === 'finished'
          ? text.agentFinished(duration(elapsed, false))
          : text.agentActivityUnknown;
  return (
    <span className="agent-activity" data-activity={kind}>
      {kind === 'queued' && (
        <span className="agent-activity-mark" aria-hidden>
          <DashedCircle />{' '}
        </span>
      )}
      {(kind === 'finished' || kind === 'unknown') && (
        <span className="agent-activity-mark" aria-hidden>
          ✓{' '}
        </span>
      )}
      {kind === 'working' && <BusyMark className="agent-activity-spinner" />}
      {kind === 'sent' ? (
        text.agentSent
      ) : (
        <>
          {kind === 'unknown' && 'sent · '}
          {kind === 'working' ? (
            // The line's own mark animates; the name stays plain so the chip's dot adds nothing.
            <span className="agent-activity-name">{name}</span>
          ) : (
            <AgentChip name={name} />
          )}
          {kind !== 'unknown' && ' '}
          {detail}
        </>
      )}
    </span>
  );
}
