/** Closed work values and detached actions: an ordinary page's summary and task list. No target
 * access or publication authority. The admitted writer supplies attribution, and the
 * work-in-progress limit is the write owner's admission rule over the whole space, never a rule
 * of this value, so no published state is ever invalid for it. Mirrors `work.rs`. */
import {
  byteLength,
  canonical,
  coreIdentity,
  id,
  keys,
  multiline,
  nonempty,
  object,
  plain,
  requireValue,
} from './closed.js';
import { validateSummary, type SummaryLines } from './summary-lines.js';

export const TASK_STATES = ['todo', 'in-progress', 'blocked', 'in-review', 'done'] as const;
export type TaskState = (typeof TASK_STATES)[number];
export type Assignee =
  | { kind: 'agent'; name: string; id?: string }
  | { kind: 'person'; name: string };
export type Criterion =
  | { kind: 'test'; name: string }
  | { kind: 'check'; command: string; result?: string }
  | { kind: 'person'; who: string };
export interface Task {
  title: string;
  state: TaskState;
  detail?: string;
  /** Present exactly when the task is blocked. */
  reason?: string;
  assignee?: Assignee;
  acceptance?: Criterion[];
  /** The task waits for the owner's sign-off; present only when true. */
  needsSignoff?: true;
  externalId?: string;
  /** The page this task follows; excludes `externalId`, and the task's state is then its page's. */
  child?: string;
}
export interface Work {
  version: 1;
  /** The number the next task takes; a removed number is never handed out again. */
  next: number;
  tasks: Record<string, Task>;
  order: string[];
  summary?: SummaryLines;
}
/** Where a page stands, computed from its tasks and never stored. */
export interface Derived {
  /** `discussion` with no task, `in-progress` while any is not done, `done` when all are. */
  stage: 'discussion' | 'in-progress' | 'done';
  done: number;
  total: number;
  /** The assignees of in-progress tasks, each once, in task order. */
  working: Assignee[];
  /** Flagged tasks waiting in review. The owner answers the flag from the band (sign off or send back). */
  signoff: number;
  /** What each task that follows a page shows, when that page's stage is known; present only when
   * some task does. The other tasks show their own state. */
  states?: Record<string, TaskState>;
}
export const WORK_TASKS = 256;
export const WORK_BYTES = 128 * 1024;
export const WORK_ID_MAX = 9_999_999;
const TITLE_POINTS = 120;
const DETAIL_BYTES = 4096;
const REASON_BYTES = 256;
const NAME_BYTES = 128;
const EXTERNAL_ID_BYTES = 128;
const ACCEPTANCE = 8;
const CRITERION_BYTES = 256;
const RESULT_BYTES = 512;

/** A task ID is `T-` and a decimal number from 1, without a leading zero. */
export function taskNumber(value: string): number | undefined {
  const digits = /^T-([1-9][0-9]{0,6})$/.exec(value)?.[1];
  return digits === undefined ? undefined : Number(digits);
}
export function validateTaskId(value: unknown): asserts value is string {
  requireValue(typeof value === 'string' && taskNumber(value) !== undefined);
}
function title(value: unknown) {
  plain(value, TITLE_POINTS * 4);
  requireValue(value.length > 0 && [...value].length <= TITLE_POINTS);
}
function assignee(value: unknown) {
  const who = object(value);
  if (who.kind === 'agent') {
    keys(who, ['kind', 'name'], ['id']);
    nonempty(who.name, NAME_BYTES);
    if (Object.hasOwn(who, 'id')) coreIdentity(who.id);
  } else {
    requireValue(who.kind === 'person');
    keys(who, ['kind', 'name']);
    nonempty(who.name, NAME_BYTES);
  }
}
function criterion(value: unknown) {
  const item = object(value);
  if (item.kind === 'test') {
    keys(item, ['kind', 'name']);
    nonempty(item.name, CRITERION_BYTES);
  } else if (item.kind === 'check') {
    keys(item, ['kind', 'command'], ['result']);
    nonempty(item.command, CRITERION_BYTES);
    if (Object.hasOwn(item, 'result')) nonempty(item.result, RESULT_BYTES);
  } else {
    requireValue(item.kind === 'person');
    keys(item, ['kind', 'who']);
    nonempty(item.who, NAME_BYTES);
  }
}
/** The fields a person or agent edits: everything except state, reason and assignee. */
export const EDITABLE = ['title', 'detail', 'acceptance', 'needsSignoff', 'externalId'] as const;
export function validateEditable(value: Record<string, unknown>) {
  title(value.title);
  if (Object.hasOwn(value, 'detail')) multiline(value.detail, DETAIL_BYTES);
  if (Object.hasOwn(value, 'acceptance')) {
    const list = value.acceptance;
    requireValue(Array.isArray(list) && list.length > 0 && list.length <= ACCEPTANCE);
    list.forEach(criterion);
  }
  if (Object.hasOwn(value, 'needsSignoff')) requireValue(value.needsSignoff === true);
  if (Object.hasOwn(value, 'externalId')) nonempty(value.externalId, EXTERNAL_ID_BYTES);
}
/** What a task keeps when only its title and detail are edited: `task.set` replaces the whole
 * editable set, so the rest is sent back as it is. */
export function keptByEdit(task: Task): Record<string, unknown> {
  return {
    ...(task.acceptance ? { acceptance: task.acceptance } : {}),
    ...(task.needsSignoff ? { needsSignoff: true } : {}),
    ...(task.externalId !== undefined ? { externalId: task.externalId } : {}),
  };
}
export function validateTask(value: unknown): asserts value is Task {
  const task = keys(
    value,
    ['title', 'state'],
    ['detail', 'reason', 'assignee', 'acceptance', 'needsSignoff', 'externalId', 'child'],
  );
  validateEditable(task);
  // A task links to a page or to an external issue, never both: its state has one owner.
  if (Object.hasOwn(task, 'child')) {
    id(task.child);
    requireValue(!Object.hasOwn(task, 'externalId'));
  }
  requireValue(TASK_STATES.includes(task.state as TaskState));
  // A blocked task says why, and only a blocked task does.
  if (Object.hasOwn(task, 'reason')) {
    requireValue(task.state === 'blocked');
    nonempty(task.reason, REASON_BYTES);
  } else requireValue(task.state !== 'blocked');
  if (Object.hasOwn(task, 'assignee')) assignee(task.assignee);
}
/** Measures the admitted value, without retained Yjs history or transport framing. */
export function validateWork(value: unknown): asserts value is Work {
  const work = keys(value, ['version', 'next', 'tasks', 'order'], ['summary']);
  requireValue(work.version === 1);
  requireValue(
    Number.isSafeInteger(work.next) &&
      (work.next as number) >= 1 &&
      (work.next as number) <= WORK_ID_MAX + 1,
  );
  const tasks = object(work.tasks);
  requireValue(Object.keys(tasks).length <= WORK_TASKS);
  for (const [key, task] of Object.entries(tasks)) {
    // Every task number is below `next`: a removed number is never handed out again.
    const number = taskNumber(key);
    requireValue(number !== undefined && number < (work.next as number));
    validateTask(task);
  }
  requireValue(Array.isArray(work.order) && work.order.length === Object.keys(tasks).length);
  const seen = new Set<string>();
  for (const entry of work.order) {
    validateTaskId(entry);
    requireValue(Object.hasOwn(tasks, entry) && !seen.has(entry));
    seen.add(entry);
  }
  if (Object.hasOwn(work, 'summary')) validateSummary(work.summary);
  requireValue(byteLength(canonical(work)) <= WORK_BYTES);
}
/** What a task shows while its page is at `stage`: nothing yet is `todo`, work under way is
 * `in-progress`, and a finished page is `done`, or `in-review` when the task waits for sign-off. */
export function linkedState(stage: string, needsSignoff: boolean): TaskState | undefined {
  if (stage === 'discussion') return 'todo';
  if (stage === 'in-progress') return 'in-progress';
  if (stage === 'done') return needsSignoff ? 'in-review' : 'done';
  return undefined;
}
/** `stageOf` answers a linked page's stage. A page it cannot answer for (deleted, unreadable)
 * leaves its task showing its own state. */
export function derivedWork(
  work: unknown,
  stageOf: (page: string) => string | undefined = () => undefined,
): Derived {
  validateWork(work);
  let done = 0;
  let signoff = 0;
  const working: Assignee[] = [];
  const states: Record<string, TaskState> = {};
  for (const key of work.order) {
    const task = work.tasks[key];
    const stage = task.child === undefined ? undefined : stageOf(task.child);
    const followed =
      stage === undefined ? undefined : linkedState(stage, task.needsSignoff === true);
    if (followed !== undefined) states[key] = followed;
    const state = followed ?? task.state;
    if (state === 'done') done += 1;
    else if (state === 'in-progress') {
      const who = task.assignee;
      if (who && !working.some((entry) => canonical(entry) === canonical(who))) working.push(who);
    } else if (state === 'in-review' && task.needsSignoff) signoff += 1;
  }
  const total = work.order.length;
  const stage = total === 0 ? 'discussion' : done === total ? 'done' : 'in-progress';
  return {
    stage,
    done,
    total,
    working,
    signoff,
    ...(Object.keys(states).length > 0 ? { states } : {}),
  };
}
