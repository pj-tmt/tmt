import { Bot } from 'lucide-react';
import { Fragment, type ReactNode } from 'react';
import { text } from '../strings.js';
import './agent-chip.css';

/** The Remote directory's advisory activity of an agent; only `working` changes the chip. */
export type AgentActivity = 'working' | 'idle' | 'unknown' | 'ended';
/** What the composer knows about a mentioned agent; cosmetic and never routing. */
export type AgentChipState = 'checking' | 'not-found' | 'offline' | 'online' | 'unknown';

/**
 * The one place an agent is shown: icon, name, then muted machine and driver. `chip` is the
 * bordered form for what the reader edits (a composer mention, a question recipient) and takes a
 * trailing control as children; `inline` sits inside a sentence, a menu row or a byline. The icon
 * becomes a labelled spinner while the agent works; any other or unknown activity keeps the icon.
 */
export function AgentChip({
  name,
  machine,
  driver,
  state,
  activity,
  variant = 'inline',
  title,
  children,
}: {
  name: string;
  machine?: string;
  driver?: string;
  state?: AgentChipState;
  activity?: AgentActivity;
  variant?: 'inline' | 'chip';
  title?: string;
  children?: ReactNode;
}) {
  const working = activity === 'working';
  return (
    <span
      className="agent-chip"
      data-variant={variant}
      data-state={state}
      data-activity={activity}
      title={title}
    >
      {working ? (
        <span className="agent-chip-spinner" role="img" aria-label={text.agentWorking} />
      ) : (
        <Bot className="agent-chip-icon" aria-hidden />
      )}
      <span className="agent-chip-name">{name}</span>
      {machine && <span className="agent-chip-machine"> · {machine}</span>}
      {driver && <span className="agent-chip-driver"> · {driver}</span>}
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
