/** A page's work value (`meta.work`): its direct sub-pages in order and its two-line summary. No
 * target access or publication authority; which page may be listed is the write owner's admission
 * over the whole space, never a rule of this value. Mirrors `work.rs`.
 *
 * Version 2 is the shape every write produces. Version 1 is the task-era value (#5825): a reader
 * still accepts it and projects its linked tasks as sub-pages, read only, until the owner's serve
 * rewrites it. */
import { byteLength, canonical, id, keys, object, requireValue } from './closed.js';
import { validateSummary, type SummaryLines } from './summary-lines.js';

export interface Work {
  version: 2;
  /** The direct sub-pages, in the page's order. */
  order: string[];
  summary?: SummaryLines;
}
/** The task-era value, read only: a reader takes its linked tasks as sub-pages. */
export interface LegacyWork {
  version: 1;
  next: number;
  tasks: Record<string, { child?: string } & Record<string, unknown>>;
  order: string[];
  summary?: SummaryLines;
}
export type StoredWork = Work | LegacyWork;
export const SUBPAGES = 256;
/** Legacy-migration-only: the most tasks a task-era value held. */
const WORK_TASKS = 256;
export const WORK_BYTES = 128 * 1024;

/** A task-era task ID, `T-` and a decimal number from 1, without a leading zero. Only legacy
 * values and hints still carry one. */
export function taskNumber(value: string): number | undefined {
  const digits = /^T-([1-9][0-9]{0,6})$/.exec(value)?.[1];
  return digits === undefined ? undefined : Number(digits);
}

/** A stored value of either version, checked as far as reading it needs. */
export function validateWork(value: unknown): asserts value is StoredWork {
  const version = object(value).version;
  if (version === 2) {
    const work = keys(value, ['version', 'order'], ['summary']);
    requireValue(Array.isArray(work.order) && work.order.length <= SUBPAGES);
    const seen = new Set<string>();
    for (const entry of work.order) {
      id(entry);
      requireValue(!seen.has(entry));
      seen.add(entry);
    }
    if (Object.hasOwn(work, 'summary')) validateSummary(work.summary);
  } else {
    requireValue(version === 1);
    const work = keys(value, ['version', 'next', 'tasks', 'order'], ['summary']);
    requireValue(Number.isSafeInteger(work.next));
    const tasks = object(work.tasks);
    requireValue(
      Object.keys(tasks).length <= WORK_TASKS &&
        Array.isArray(work.order) &&
        work.order.length === Object.keys(tasks).length,
    );
    const seen = new Set<string>();
    for (const entry of work.order) {
      requireValue(typeof entry === 'string' && Object.hasOwn(tasks, entry) && !seen.has(entry));
      seen.add(entry);
      const task = object(tasks[entry]);
      if (Object.hasOwn(task, 'child')) id(task.child);
    }
    if (Object.hasOwn(work, 'summary')) validateSummary(work.summary);
  }
  requireValue(byteLength(canonical(value)) <= WORK_BYTES);
}

/** The direct sub-pages, in the page's order. A task-era value reads its linked tasks in task
 * order, each page once, and its plain tasks not at all. */
export function subpages(work: StoredWork | undefined): string[] {
  if (!work) return [];
  const pages =
    work.version === 1
      ? work.order.flatMap((key) => {
          const child = work.tasks[key]?.child;
          return child === undefined ? [] : [child];
        })
      : work.order;
  return [...new Set(pages)];
}

/** The version 2 value of a stored value of either version: the same sub-pages and summary. */
export function migratedWork(work: StoredWork): Work {
  return {
    version: 2,
    order: subpages(work),
    ...(work.summary ? { summary: work.summary } : {}),
  };
}
