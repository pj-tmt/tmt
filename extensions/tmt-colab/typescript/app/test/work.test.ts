import { expect, it } from 'vite-plus/test';
import vectors from '../../../contracts/vectors/work-v1.json';
import { byteLength, canonical } from '../src/closed.js';
import type { Attribution } from '../src/summary-lines.js';
import {
  WORK_BYTES,
  WORK_TASKS,
  applyWorkAction,
  derivedWork,
  emptyWork,
  taskNumber,
  validateWork,
  type Work,
} from '../src/work.js';

it.each(vectors.cases)('$name', (fixture) => {
  const before = JSON.stringify(fixture.document);
  const run = () =>
    'action' in fixture
      ? applyWorkAction(
          fixture.document,
          fixture.action!,
          fixture.body,
          fixture.attribution as Attribution,
        )
      : validateWork(fixture.document);
  if (!fixture.valid) expect(run).toThrow('Invalid structured value');
  else {
    const result = run();
    if ('expected' in fixture) expect(result).toEqual(fixture.expected);
    const value = result ?? fixture.document;
    expect(canonical(value)).toBe(fixture.canonical);
    if ('derived' in fixture) expect(derivedWork(value)).toEqual(fixture.derived);
  }
  expect(JSON.stringify(fixture.document)).toBe(before);
});

it('enforces the whole-value byte limit at exactly 128 KiB and the 256-task bound', () => {
  const value: Work = emptyWork();
  for (let n = 1; n <= WORK_TASKS; n++) {
    value.tasks[`T-${n}`] = { title: 't', state: 'todo' };
    value.order.push(`T-${n}`);
  }
  value.next = WORK_TASKS + 1;
  expect(() => validateWork(value)).not.toThrow();
  const overTasks = structuredClone(value);
  overTasks.tasks['T-257'] = { title: 't', state: 'todo' };
  overTasks.order.push('T-257');
  overTasks.next = 258;
  expect(() => validateWork(overTasks)).toThrow();
  let last = '';
  for (const id of value.order) {
    value.tasks[id].detail = '';
    const remaining = WORK_BYTES - byteLength(canonical(value));
    value.tasks[id].detail = 'x'.repeat(Math.min(4096, remaining));
    if (remaining <= 4096) {
      last = id;
      break;
    }
  }
  expect(byteLength(canonical(value))).toBe(WORK_BYTES);
  expect(() => validateWork(value)).not.toThrow();
  value.tasks[last].detail += 'x';
  expect(() => validateWork(value)).toThrow();
});

it('reads a task number only in its exact form', () => {
  for (const good of ['T-1', 'T-10', 'T-9999999']) expect(taskNumber(good)).toBeDefined();
  for (const bad of [
    'T-0',
    'T-01',
    'T-',
    't-1',
    'T-+1',
    'T-10000000',
    'T--1',
    '1',
    'T-1 ',
    'T-1\n',
  ])
    expect(taskNumber(bad)).toBeUndefined();
});

it('never issues a removed number again', () => {
  const attribution = { writerId: '10000000-0000-4000-8000-000000000001', timeMs: 1 };
  let value = emptyWork();
  for (const title of ['a', 'b'])
    value = applyWorkAction(value, 'task.add', { task: { title } }, attribution);
  value = applyWorkAction(value, 'task.remove', { taskId: 'T-2' }, attribution);
  value = applyWorkAction(value, 'task.add', { task: { title: 'c' } }, attribution);
  expect(value.order).toEqual(['T-1', 'T-3']);
  expect(value.next).toBe(4);
});

it('refuses unpaired surrogates and control characters the native side cannot hold either', () => {
  const task = (title: string) => ({
    version: 1,
    next: 2,
    tasks: { 'T-1': { title, state: 'todo' } },
    order: ['T-1'],
  });
  expect(() => validateWork(task('ok 🦊'))).not.toThrow();
  expect(() => validateWork(task('lone \ud800'))).toThrow();
  expect(() => validateWork(task('bell \u0007'))).toThrow();
  expect(() => validateWork(task('del \u007f'))).toThrow();
});
