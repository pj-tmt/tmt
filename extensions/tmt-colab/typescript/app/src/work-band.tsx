import { useId, useMemo, useState, type ReactNode } from 'react';
import { text } from './strings.js';
import { derivedWork, type Work } from './work.js';
import { TaskDetail, type TaskDetailSlots } from './work-detail.js';
import { Assignees } from './work-people.js';
import { WorkList } from './work-list.js';
import './work-band.css';

/** The task a page follows from another page; drawn above the summary as a way back. */
export interface WorkParent {
  pageId: string;
  taskId: string;
  title: string;
}
export interface WorkBandProps extends TaskDetailSlots {
  work: Work;
  /** Below the list and the open task: where a new task is made and a refusal is told. */
  listActions?: ReactNode;
  /** Where this page's own work sits in a parent page's task list. */
  parent?: WorkParent;
  pageHref?(pageId: string): string;
  /** A linked page's stage, when known; a task that follows it shows that state. */
  stageOf?(pageId: string): string | undefined;
}

/**
 * Where the page stands: two summary lines, progress and who works, then the tasks one step away.
 * It scrolls with the page, never sticks, and opening the list pushes the page down in place.
 */
export function WorkBand({
  work,
  listActions,
  parent,
  pageHref,
  stageOf,
  ...slots
}: WorkBandProps): ReactNode {
  const derived = useMemo(() => derivedWork(work, stageOf), [work, stageOf]);
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<string>();
  const listId = useId();
  const summary = work.summary;
  if (!summary && derived.total === 0 && !parent) return null;
  const task = selected === undefined ? undefined : work.tasks[selected];
  return (
    <section className="work-band" aria-label={text.workBand.label} data-stage={derived.stage}>
      {parent && (
        <p className="work-parent">
          <a href={pageHref?.(parent.pageId)}>
            {text.workBand.followsParent(parent.taskId, parent.title)}
          </a>
        </p>
      )}
      <div className="work-head">
        <div className="work-lines">
          {summary ? (
            <>
              <p className="work-line1">{summary.line1}</p>
              <p className="work-line2">{summary.line2}</p>
            </>
          ) : (
            <p className="work-line1">{text.workBand.stage[derived.stage]}</p>
          )}
        </div>
        {derived.total > 0 && (
          <div className="work-facts">
            <span className="work-stage" data-stage={derived.stage}>
              {text.workBand.stage[derived.stage]}
            </span>
            <span className="work-progress">
              {text.workBand.progress(derived.done, derived.total)}
            </span>
            {derived.working.length > 0 && (
              <span className="work-working">
                <Assignees people={derived.working} />
              </span>
            )}
            {derived.signoff > 0 && (
              <span className="work-signoff">{text.workBand.signoff(derived.signoff)}</span>
            )}
          </div>
        )}
        {derived.total > 0 && (
          <button
            type="button"
            className="work-toggle"
            aria-expanded={open}
            aria-controls={listId}
            onClick={() => {
              setOpen(!open);
              if (open) setSelected(undefined);
            }}
          >
            {open ? text.workBand.hideTasks : text.workBand.showTasks(derived.total)}
          </button>
        )}
      </div>
      {open && (
        <div className="work-body" id={listId}>
          <WorkList
            work={work}
            states={derived.states}
            selected={selected}
            onSelect={(id) => setSelected(id === selected ? undefined : id)}
          />
          {selected !== undefined && task && (
            <TaskDetail
              id={selected}
              task={task}
              state={derived.states?.[selected] ?? task.state}
              pageHref={pageHref}
              onClose={() => setSelected(undefined)}
              {...slots}
            />
          )}
          {listActions}
        </div>
      )}
    </section>
  );
}
