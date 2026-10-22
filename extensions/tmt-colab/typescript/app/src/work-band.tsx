import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react';
import type { PageHint } from './reading-link.js';
import { text } from './strings.js';
import { derivedWork, type Work } from './work.js';
import { TaskDetail, type TaskDetailSlots } from './work-detail.js';
import { Assignees } from './work-people.js';
import { WorkList } from './work-list.js';
import './work-band.css';

/** The task of another page that follows this page; drawn above the summary as a way back. */
export interface WorkParent {
  pageId: string;
  taskId: string;
  /** The parent page's title. */
  title: string;
  /** The parent task's own title, clipped by the owner; absent from an older owner. */
  taskTitle?: string;
}
export interface WorkBandProps extends TaskDetailSlots {
  work: Work;
  /** Below the list and the open task: where a new task is made and a refusal is told. */
  listActions?: ReactNode;
  /** Where this page's own work sits in a parent page's task list. */
  parent?: WorkParent;
  /** The address of a page, optionally at one cosmetic hint (a task). */
  pageHref?(pageId: string, hint?: PageHint): string;
  /** A task of this page to open with the list, from a link to it (`?task=`); one this page does
   * not have opens nothing. Applied once per page, so a later close is never undone. */
  focusTask?: string;
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
  focusTask,
  stageOf,
  ...slots
}: WorkBandProps): ReactNode {
  const derived = useMemo(() => derivedWork(work, stageOf), [work, stageOf]);
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<string>();
  const listId = useId();
  const focused = useRef<string | undefined>(undefined);
  const detail = useRef<HTMLDivElement>(null);
  // A link to one task opens the list with it selected, once the page has that task (its work
  // can arrive after the page opens). A task it does not have changes nothing, and says nothing.
  useEffect(() => {
    if (focusTask === undefined || focused.current === focusTask || !work.tasks[focusTask]) return;
    focused.current = focusTask;
    setOpen(true);
    setSelected(focusTask);
  }, [focusTask, work]);
  useEffect(() => {
    if (selected !== undefined && selected === focused.current)
      detail.current?.scrollIntoView({ block: 'nearest' });
  }, [selected, open]);
  const summary = work.summary;
  if (!summary && derived.total === 0 && !parent) return null;
  const task = selected === undefined ? undefined : work.tasks[selected];
  return (
    <section className="work-band" aria-label={text.workBand.label} data-stage={derived.stage}>
      {parent && (
        <p className="work-parent">
          <a href={pageHref?.(parent.pageId, { task: parent.taskId })}>
            {text.workBand.parentTask(parent.title, parent.taskId, parent.taskTitle)}
          </a>
        </p>
      )}
      {(summary || derived.total > 0 || listActions) && (
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
      )}
      {derived.total === 0 && listActions && <div className="work-body">{listActions}</div>}
      {open && (
        <div className="work-body" id={listId} ref={detail}>
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
