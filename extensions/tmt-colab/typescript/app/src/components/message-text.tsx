import { useContext } from 'react';
import { AgentPresentation } from '../agent-directory.js';
import { AgentChip } from './agent-chip.js';
/** Cosmetic tokens only; sender labels and message text grant no recipient authority. The chip's
 * icon takes the place of the typed `@`.
 * A name with a space stays one chip when the caller or the presentation directory knows it. */
export function MessageText({ value, names = [] }: { value: string; names?: readonly string[] }) {
  const directory = useContext(AgentPresentation);
  const labels = [
    ...new Set([...names, ...directory.map((agent) => agent.agentName)].filter(Boolean)),
  ]
    .sort((a, b) => b.length - a.length)
    .map((name) => name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const known = labels.length ? `(?:${labels.join('|')})(?=$|[^\\p{L}\\p{N}_-])|` : '';
  const token = `${known}[\\p{L}\\p{N}_](?:[\\p{L}\\p{N}_.-]*[\\p{L}\\p{N}_-])?`;
  return value
    .split(new RegExp(`(?<![A-Za-z0-9_.+\\-@＠])([@＠](?:${token}))`, 'u'))
    .map((part, index) =>
      index % 2 === 1 ? <AgentChip name={part.slice(1)} key={index} /> : part,
    );
}
