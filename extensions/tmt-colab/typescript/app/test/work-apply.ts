/** The TypeScript side of the cross-language work vectors (`contracts/vectors/work-v1.json`, Rust
 * `work.rs`): applying an action to a work value. Production never applies an action in the
 * browser (the serve prepares it natively), so this lives with the tests that need it. Mirrors
 * `work.rs`. */
import { id, keys, object, requireValue } from '../src/closed.js';
import { setSummary, type Attribution } from '../src/summary-lines.js';
import {
  EDITABLE,
  WORK_ID_MAX,
  validateEditable,
  validateTaskId,
  validateWork,
  type Task,
  type Work,
} from '../src/work.js';

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
      'task.link': [['taskId', 'pageId'], []],
      'task.unlink': [['taskId'], []],
      'task.remove': [['taskId'], []],
    };
    requireValue(Object.hasOwn(spec, action));
    keys(body, ...spec[action]);
    validateTaskId(body.taskId);
    const key = body.taskId;
    requireValue(Object.hasOwn(next.tasks, key));
    const task = next.tasks[key] as unknown as Record<string, unknown>;
    if (action === 'task.set') {
      // The whole editable part is replaced: a field the body omits is cleared.
      validateEditable(body);
      for (const field of EDITABLE)
        if (Object.hasOwn(body, field)) task[field] = structuredClone(body[field]);
        else delete task[field];
    } else if (action === 'task.state') {
      // A linked task's state follows its page: nothing sets it.
      requireValue(!Object.hasOwn(task, 'child'));
      task.state = body.state;
      if (Object.hasOwn(body, 'reason')) task.reason = body.reason;
      else delete task.reason;
    } else if (action === 'task.assign') {
      if (Object.hasOwn(body, 'assignee')) task.assignee = structuredClone(body.assignee);
      else delete task.assignee;
    } else if (action === 'task.link') {
      id(body.pageId);
      // Another page replaces nothing: the task is unlinked first.
      requireValue(!Object.hasOwn(task, 'child') || task.child === body.pageId);
      task.child = body.pageId;
    } else if (action === 'task.unlink') {
      delete task.child;
    } else {
      delete next.tasks[key];
      next.order = next.order.filter((entry) => entry !== key);
    }
  }
  validateWork(next);
  return next;
}
