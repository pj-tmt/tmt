import { AgentChip } from './components/agent-chip.js';
import { text } from './strings.js';
import type { Assignee, Task, TaskState } from './work.js';

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

/** A task waits for the owner's sign-off while it is in review and flagged. */
export const waitsForSignoff = (task: Task, state: TaskState) =>
  state === 'in-review' && task.needsSignoff === true;

export function SignoffMark() {
  return (
    <span className="work-signoff-mark">
      <span aria-hidden="true">◆</span> {text.workBand.signoffMark}
    </span>
  );
}
