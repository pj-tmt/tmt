import type { ReactNode } from 'react';
import { text } from './strings.js';
import type { Criterion, Task, TaskState } from './work.js';
import { Assignees } from './work-people.js';

/** Where later changes put their controls; nothing here writes, so each slot is empty today. */
export interface TaskDetailSlots {
  /** Controls beside the task's own: edit, state, assign, sign-off, convert (#4118). */
  actions?(task: Task, id: string): ReactNode;
}

function criterion(value: Criterion): string {
  if (value.kind === 'test') return text.workBand.criterionTest(value.name);
  if (value.kind === 'check') {
    return value.result === undefined
      ? value.command
      : text.workBand.criterionChecked(value.command, value.result);
  }
  return text.workBand.criterionPerson(value.who);
}

export function TaskDetail({
  id,
  task,
  state,
  pageHref,
  onClose,
  actions,
}: TaskDetailSlots & {
  id: string;
  task: Task;
  state: TaskState;
  pageHref?(pageId: string): string;
  onClose(): void;
}) {
  return (
    <section className="work-detail" aria-label={text.workBand.detailLabel(id)}>
      <header>
        <h3>
          <span className="work-id">{id}</span> {task.title}
        </h3>
        <button type="button" className="work-close" onClick={onClose}>
          {text.workBand.close}
        </button>
      </header>
      <p className="work-detail-state" data-state={state}>
        {text.workBand.taskState[state]}
        {state === 'blocked' && task.reason ? ` · ${task.reason}` : ''}
        {task.needsSignoff && state === 'in-review' ? ` · ${text.workBand.needsSignoff}` : ''}
      </p>
      {task.child !== undefined && (
        <p className="work-follows">
          <a href={pageHref?.(task.child)}>{text.workBand.followsPage}</a>
        </p>
      )}
      {task.assignee && (
        <p className="work-detail-assignee">
          <Assignees people={[task.assignee]} />
        </p>
      )}
      {task.detail && <p className="work-detail-text">{task.detail}</p>}
      {task.acceptance && task.acceptance.length > 0 && (
        <ul className="work-acceptance" aria-label={text.workBand.acceptance}>
          {task.acceptance.map((entry, index) => (
            <li key={index}>{criterion(entry)}</li>
          ))}
        </ul>
      )}
      {task.externalId !== undefined && (
        <p className="work-external">{text.workBand.external(task.externalId)}</p>
      )}
      {actions && <div className="work-actions">{actions(task, id)}</div>}
    </section>
  );
}
