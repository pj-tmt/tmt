import { text } from './strings.js';
import type { TaskState, Work } from './work.js';
import { Assignees, SignoffMark, waitsForSignoff } from './work-people.js';

export function WorkList({
  work,
  states,
  selected,
  onSelect,
}: {
  work: Work;
  /** What each task that follows a page shows; the others show their own state. */
  states?: Record<string, TaskState>;
  selected?: string;
  onSelect(id: string): void;
}) {
  return (
    <ol className="work-list">
      {work.order.map((id) => {
        const task = work.tasks[id]!;
        const state = states?.[id] ?? task.state;
        return (
          <li key={id} className="work-row" data-state={state} data-selected={id === selected}>
            <button
              type="button"
              className="work-row-button"
              aria-expanded={id === selected}
              onClick={() => onSelect(id)}
            >
              <span className="work-id">{id}</span>
              <span className="work-title">
                {task.title}
                {waitsForSignoff(task, state) && <SignoffMark />}
              </span>
              <span className="work-state">{text.workBand.taskState[state]}</span>
              <span className="work-assignee">
                {task.assignee && <Assignees people={[task.assignee]} />}
              </span>
            </button>
          </li>
        );
      })}
    </ol>
  );
}
