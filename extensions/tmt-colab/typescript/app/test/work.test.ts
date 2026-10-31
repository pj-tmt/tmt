import { expect, it } from 'vite-plus/test';
import vectors from '../../../contracts/vectors/work-v2.json';
import { canonical } from '../src/closed.js';
import type { Attribution } from '../src/summary-lines.js';
import { SUBPAGES, subpages, taskNumber, validateWork, type StoredWork } from '../src/work.js';
import { applyWorkAction, emptyWork } from './work-apply.js';

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
    const value = (result ?? fixture.document) as StoredWork;
    expect(canonical(value)).toBe(fixture.canonical);
    if ('children' in fixture) expect(subpages(value)).toEqual(fixture.children);
  }
  expect(JSON.stringify(fixture.document)).toBe(before);
});

it('bounds the list at 256 sub-pages', () => {
  const value = emptyWork();
  for (let n = 0; n < SUBPAGES; n++)
    value.order.push(`20000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`);
  expect(() => validateWork(value)).not.toThrow();
  const over = `20000000-0000-4000-8000-${SUBPAGES.toString(16).padStart(12, '0')}`;
  expect(() =>
    applyWorkAction(
      value,
      'subpage.attach',
      { pageId: over },
      {
        writerId: '10000000-0000-4000-8000-000000000001',
        timeMs: 1,
      },
    ),
  ).toThrow();
});

it('reads a task-era task number only in its exact form', () => {
  for (const good of ['T-1', 'T-10', 'T-9999999']) expect(taskNumber(good)).toBeDefined();
  for (const bad of ['T-0', 'T-01', 'T-', 't-1', 'T-+1', 'T-10000000'])
    expect(taskNumber(bad)).toBeUndefined();
});
