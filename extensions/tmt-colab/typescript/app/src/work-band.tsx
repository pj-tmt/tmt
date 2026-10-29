import { ChevronDown, ChevronUp } from 'lucide-react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import {
  useEffect,
  useLayoutEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import type { PageHint } from './reading-link.js';
import { text } from './strings.js';
import { derivedWork, type Work } from './work.js';
import { TaskDetail, type TaskDetailSlots } from './work-detail.js';
import type { PageProgress, PageState } from './bootstrap.js';
import { WorkList } from './work-list.js';
import './work-band.css';

/** The task of another page that follows this page; drawn in the expanded panel as a way back. */
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
  onSize?(height: number): void;
  pageState?: PageState | null;
  progress?: PageProgress | null;
  stateControl?: ReactNode;
  subpages?: readonly { id: string; title?: string; state?: PageState | null }[];
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
 * The manual page state and native progress, with direct items and task controls one step away.
 * It scrolls with the page, never sticks, and opening the list pushes the page down in place.
 */
export function WorkBand({
  work,
  onSize,
  listActions,
  parent,
  pageHref,
  focusTask,
  stageOf,
  pageState,
  progress,
  stateControl,
  subpages = [
    ...new Set(
      work.order.map((id) => work.tasks[id]?.child).filter((id): id is string => id !== undefined),
    ),
  ].map((id) => ({ id, title: undefined, state: undefined })),
  ...slots
}: WorkBandProps): ReactNode {
  const derived = useMemo(() => derivedWork(work, stageOf), [work, stageOf]);
  const [open, setOpen] = useState(false);
  const [tasksOpen, setTasksOpen] = useState(false);
  const [selected, setSelected] = useState<string>();
  const listId = useId();
  const section = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const element = section.current;
    if (!element || !onSize) return;
    const measure = () => onSize(element.getBoundingClientRect().height);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [onSize]);
  const focused = useRef<string | undefined>(undefined);
  const detail = useRef<HTMLDivElement>(null);
  // A link to one task opens the list with it selected, once the page has that task (its work
  // can arrive after the page opens). A task it does not have changes nothing, and says nothing.
  useEffect(() => {
    if (focusTask === undefined || focused.current === focusTask || !work.tasks[focusTask]) return;
    focused.current = focusTask;
    setOpen(true);
    setTasksOpen(true);
    setSelected(focusTask);
  }, [focusTask, work]);
  useEffect(() => {
    if (selected !== undefined && selected === focused.current)
      detail.current?.scrollIntoView({ block: 'nearest' });
  }, [selected, open]);
  const task = selected === undefined ? undefined : work.tasks[selected];
  const tasks = { ...work, order: work.order.filter((id) => work.tasks[id]?.child === undefined) };
  const t = text.workBand;
  const buckets = progress
    ? ([
        ['done', progress.done],
        ['in-progress', progress.inProgress],
        ['todo', progress.todo],
        ['unknown', progress.unknown],
      ] as const)
    : [];
  return (
    <section ref={section} className="work-band" aria-label={t.label}>
      <div className="work-head">
        {stateControl ?? (
          <span className={`work-page-state ${ui.countChip}`} data-state={pageState ?? 'unknown'}>
            {pageState ? t.taskState[pageState] : t.stateUnavailable}
          </span>
        )}
        {progress ? (
          <>
            <div
              className="work-segments"
              role="img"
              aria-label={buckets
                .map(
                  ([state, count]) =>
                    `${state === 'unknown' ? t.unknown : t.taskState[state]} ${count}`,
                )
                .join(', ')}
            >
              {buckets.map(
                ([state, count]) =>
                  count > 0 && <span key={state} data-state={state} style={{ flexGrow: count }} />,
              )}
            </div>
            <span className="work-progress">{t.progress(progress.done, progress.total)}</span>
          </>
        ) : (
          <span className="work-progress">{t.progressUnavailable}</span>
        )}
        <button
          type="button"
          className="work-toggle"
          aria-label={open ? t.collapsePanel : t.expandPanel}
          aria-expanded={open}
          aria-controls={listId}
          onClick={() => setOpen(!open)}
        >
          {open ? (
            <ChevronUp size={16} aria-hidden="true" />
          ) : (
            <ChevronDown size={16} aria-hidden="true" />
          )}
        </button>
      </div>
      {open && (
        <div className="work-body" id={listId} ref={detail}>
          {parent && (
            <p className="work-parent">
              <a href={pageHref?.(parent.pageId, { task: parent.taskId })}>
                {t.parentTask(parent.title, parent.taskId, parent.taskTitle)}
              </a>
            </p>
          )}
          {subpages.length > 0 && <h3 className="work-items-heading">{t.subpages}</h3>}
          {subpages.length > 0 && (
            <ul className="work-subpages" aria-label={t.subpages}>
              {subpages.map((child) => (
                <li key={child.id}>
                  {pageHref ? (
                    <a href={pageHref(child.id)}>{child.title ?? t.unknownPage}</a>
                  ) : (
                    <span>{child.title ?? t.unknownPage}</span>
                  )}
                  <span>{child.state ? t.taskState[child.state] : t.unknown}</span>
                </li>
              ))}
            </ul>
          )}
          {progress && (
            <div className="work-legend">
              {buckets
                .filter(([state, count]) => state !== 'unknown' || count > 0)
                .map(([state, count]) => (
                  <span key={state} data-state={state}>
                    {state === 'unknown' ? t.unknown : t.taskState[state]} {count}
                  </span>
                ))}
            </div>
          )}
          {subpages.length > 0 && (
            <button
              type="button"
              className="work-toggle"
              aria-expanded={tasksOpen}
              onClick={() => setTasksOpen(!tasksOpen)}
            >
              {t.tasks} {tasks.order.length}
            </button>
          )}
          {(subpages.length === 0 || tasksOpen) && (
            <div className="work-tasks">
              <WorkList
                work={tasks}
                states={derived.states}
                selected={selected}
                onSelect={(id) => {
                  setTasksOpen(true);
                  setSelected(id === selected ? undefined : id);
                }}
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
        </div>
      )}
    </section>
  );
}
