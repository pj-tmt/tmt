/** Closed work values and detached actions: an ordinary page's summary and task list. No target
 * access or publication authority. The admitted writer supplies attribution, and the
 * work-in-progress limit is the write owner's admission rule over the whole space, never a rule
 * of this value, so no published state is ever invalid for it. Mirrors `work.rs`. */
import {
  byteLength,
  canonical,
  coreIdentity,
  keys,
  multiline,
  nonempty,
  object,
  plain,
  requireValue,
} from './closed.js';
import {
  setSummary,
  validateSummary,
  type Attribution,
  type SummaryLines,
} from './summary-lines.js';

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
  /** Display only until the owner can act on it; present only when true. */
  needsSignoff?: true;
  externalId?: string;
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
  /** Flagged tasks waiting in review. The flag is display only until the owner can act on it. */
  signoff: number;
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
function taskId(value: unknown): asserts value is string {
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
const EDITABLE = ['title', 'detail', 'acceptance', 'needsSignoff', 'externalId'] as const;
function editable(value: Record<string, unknown>) {
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
export function validateTask(value: unknown): asserts value is Task {
  const task = keys(
    value,
    ['title', 'state'],
    ['detail', 'reason', 'assignee', 'acceptance', 'needsSignoff', 'externalId'],
  );
  editable(task);
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
    taskId(entry);
    requireValue(Object.hasOwn(tasks, entry) && !seen.has(entry));
    seen.add(entry);
  }
  if (Object.hasOwn(work, 'summary')) validateSummary(work.summary);
  requireValue(byteLength(canonical(work)) <= WORK_BYTES);
}
/** A page that has had no work yet carries no `meta.work`; its first action starts from this. */
export function emptyWork(): Work {
  return { version: 1, next: 1, tasks: {}, order: [] };
}
/** Produces a detached result or refuses the whole action; the base is not mutated.
 * `attribution` comes from the write owner, never from the action body. The work-in-progress limit
 * is not checked here: the write owner admits an assignment before it prepares one. */
export function applyWorkAction(
  base: unknown,
  action: string,
  input: unknown,
  attribution: Attribution,
): Work {
  validateWork(base);
  const next = structuredClone(base);
  const body = object(input);
  if (action === 'summary.set') next.summary = setSummary(body, attribution);
  else if (action === 'summary.clear') {
    keys(body, []);
    delete next.summary;
  } else if (action === 'task.add') {
    keys(body, ['task']);
    const task = keys(
      body.task,
      ['title'],
      ['detail', 'assignee', 'acceptance', 'needsSignoff', 'externalId'],
    );
    requireValue(next.next <= WORK_ID_MAX);
    const key = `T-${next.next}`;
    next.tasks[key] = { ...structuredClone(task), state: 'todo' } as Task;
    next.order.push(key);
    next.next += 1;
  } else {
    const spec: Record<string, [string[], string[]]> = {
      'task.set': [
        ['taskId', 'title'],
        ['detail', 'acceptance', 'needsSignoff', 'externalId'],
      ],
      'task.state': [['taskId', 'state'], ['reason']],
      'task.assign': [['taskId'], ['assignee']],
      'task.remove': [['taskId'], []],
    };
    requireValue(Object.hasOwn(spec, action));
    keys(body, ...spec[action]);
    taskId(body.taskId);
    const key = body.taskId;
    requireValue(Object.hasOwn(next.tasks, key));
    const task = next.tasks[key] as unknown as Record<string, unknown>;
    if (action === 'task.set') {
      // The whole editable part is replaced: a field the body omits is cleared.
      editable(body);
      for (const field of EDITABLE)
        if (Object.hasOwn(body, field)) task[field] = structuredClone(body[field]);
        else delete task[field];
    } else if (action === 'task.state') {
      task.state = body.state;
      if (Object.hasOwn(body, 'reason')) task.reason = body.reason;
      else delete task.reason;
    } else if (action === 'task.assign') {
      if (Object.hasOwn(body, 'assignee')) task.assignee = structuredClone(body.assignee);
      else delete task.assignee;
    } else {
      delete next.tasks[key];
      next.order = next.order.filter((entry) => entry !== key);
    }
  }
  validateWork(next);
  return next;
}
export function derivedWork(work: unknown): Derived {
  validateWork(work);
  let done = 0;
  let signoff = 0;
  const working: Assignee[] = [];
  for (const key of work.order) {
    const task = work.tasks[key];
    if (task.state === 'done') done += 1;
    else if (task.state === 'in-progress') {
      const who = task.assignee;
      if (who && !working.some((entry) => canonical(entry) === canonical(who))) working.push(who);
    } else if (task.state === 'in-review' && task.needsSignoff) signoff += 1;
  }
  const total = work.order.length;
  const stage = total === 0 ? 'discussion' : done === total ? 'done' : 'in-progress';
  return { stage, done, total, working, signoff };
}
