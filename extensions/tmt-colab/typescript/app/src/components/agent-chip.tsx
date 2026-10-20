import { Bot } from 'lucide-react';
import { createContext, Fragment, useContext, type ReactNode } from 'react';
import type { ActivityAgent } from '../agent-activity.js';
import { text } from '../strings.js';
import './agent-chip.css';

/** The Remote directory's advisory activity of an agent; only `working` changes the chip. */
export type AgentActivity = 'working' | 'idle' | 'unknown' | 'ended';
/** Fresh page-scoped presentation only, supplied by the existing Ask activity observer. */
export const AgentChipActivity = createContext<readonly ActivityAgent[]>([]);
/** What the composer knows about a mentioned agent; cosmetic and never routing. */
export type AgentChipState = 'checking' | 'not-found' | 'offline' | 'online' | 'unknown';

/**
 * The one place an agent is shown: icon, name, then the muted machine. `chip` is the
 * bordered form for what the reader edits (a composer mention, a question recipient) and takes a
 * trailing control as children; `inline` sits inside a sentence, a menu row or a byline. While the
 * agent works the icon carries a small status dot and the name gains a hidden ", working"; the chip
 * never animates, and any other or unknown activity shows the plain icon (#3978).
 */
export function AgentChip({
  name,
  machine,
  state,
  activity,
  variant = 'inline',
  title,
  children,
}: {
  name: string;
  machine?: string;
  state?: AgentChipState;
  activity?: AgentActivity;
  variant?: 'inline' | 'chip';
  title?: string;
  children?: ReactNode;
}) {
  const candidates = useContext(AgentChipActivity).filter(
    (row) => row.name === name.replace(/^@/, ''),
  );
  const rows =
    candidates.length > 1 && machine !== undefined
      ? candidates.filter((row) => row.machineName === machine)
      : candidates;
  const currentActivity = activity ?? (rows.length === 1 ? rows[0]!.state : undefined);
  const working = currentActivity === 'working';
  return (
    <span
      className="agent-chip"
      data-variant={variant}
      data-state={state}
      data-activity={currentActivity}
      title={working ? `${title ?? name}, ${text.agentWorking}` : title}
    >
      <span className="agent-chip-mark" data-working={working || undefined} aria-hidden>
        <Bot className="agent-chip-icon" />
      </span>
      <span className="agent-chip-name">{name}</span>
      {working && <span className="agent-chip-state">, {text.agentWorking}</span>}
      {machine && <span className="agent-chip-machine"> · {machine}</span>}
      {children}
    </span>
  );
}

/** A sentence template with `{agent}` where the agent goes; each marker becomes an inline chip. */
export function agentSentence(template: string, ...names: string[]): ReactNode {
  const parts = template.split('{agent}');
  return parts.map((part, index) => (
    <Fragment key={index}>
      {part}
      {index < parts.length - 1 && <AgentChip name={names[index] ?? ''} />}
    </Fragment>
  ));
}

/** A sentence template with `{agents}` where an already built list of chips goes. */
export function agentsSentence(template: string, agents: ReactNode): ReactNode {
  const [before, after = ''] = template.split('{agents}');
  return (
    <>
      {before}
      {agents}
      {after}
    </>
  );
}

/** Several agents in running text, as inline chips separated by commas. */
export function AgentList({ names }: { names: readonly string[] }) {
  return names.map((name, index) => (
    <Fragment key={`${index}:${name}`}>
      {index > 0 && ', '}
      <AgentChip name={name} />
    </Fragment>
  ));
}
