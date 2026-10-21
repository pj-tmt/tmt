import { AgentChip } from './components/agent-chip.js';
import type { Assignee } from './work.js';

export function Assignees({ people }: { people: readonly Assignee[] }) {
  return (
    <>
      {people.map((who, index) => (
        <span className="work-who" key={`${who.kind}:${who.name}:${index}`}>
          {who.kind === 'agent' ? <AgentChip name={who.name} /> : who.name}
        </span>
      ))}
    </>
  );
}
