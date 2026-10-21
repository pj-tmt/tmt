import { expect, it } from 'vite-plus/test';
import { validateHomeSearch } from '../src/home.js';
import { DEFAULT_VIEW, takeStage } from '../src/list-stage.js';
import { filterQuery } from '../src/page-list-search.js';

it('names every key, so one it cannot read replaces the raw value the route merges under it', () => {
  const dropped = validateHomeSearch({
    project: 'design',
    scope: ['design'],
    by: { name: 'Stu' },
    q: 123,
    sort: 1,
    archived: 1,
    needs: 1,
    other: 'kept by the route, never read',
  });
  expect(Object.keys(dropped).sort()).toEqual([
    'archived',
    'by',
    'needs',
    'project',
    'q',
    'scope',
    'sort',
    'stage',
  ]);
  expect(Object.values(dropped).every((value) => value === undefined)).toBe(true);
});

it('keeps what it can read and bounds it', () => {
  expect(
    validateHomeSearch({
      project: ['design', 7, 'design', 'q4 plan'],
      scope: 'q4 plan',
      by: ['Stu', '', 'Ana\u0007'],
      q: '  notes  ',
      sort: 'title',
      archived: 'true',
      needs: true,
    }),
  ).toEqual({
    project: ['design', 'q4 plan'],
    scope: 'q4 plan',
    by: ['Stu'],
    q: 'notes',
    sort: 'title',
    archived: true,
    needs: true,
  });
});

it('reads the needs-you filter from a copied link (true or "true") and nothing else', () => {
  expect(validateHomeSearch({ needs: 'true' }).needs).toBe(true);
  expect(validateHomeSearch({ needs: true }).needs).toBe(true);
  for (const value of [false, 'false', 'yes', 1, null, ['true']])
    expect(validateHomeSearch({ needs: value }).needs).toBeUndefined();
});

it('reads the scope as one project name and nothing else', () => {
  expect(validateHomeSearch({ scope: 'design' }).scope).toBe('design');
  for (const value of ['', ' lead', 'x'.repeat(33), 'tab\there', 7, null, ['design'], { a: 1 }])
    expect(validateHomeSearch({ scope: value }).scope).toBeUndefined();
});

it('reads the stage tab from a copied link, keeping only a value that is not the default', () => {
  const other = ['in-progress', 'discussion', 'done', 'all'].filter(
    (name) => name !== DEFAULT_VIEW,
  );
  for (const stage of other) expect(validateHomeSearch({ stage }).stage).toBe(stage);
  expect(validateHomeSearch({ stage: DEFAULT_VIEW }).stage).toBeUndefined();
  for (const value of ['', 'Done', 'blocked', 'archived', 7, null, ['done'], { a: 1 }])
    expect(validateHomeSearch({ stage: value }).stage).toBeUndefined();
});

it('takes the last stage: word out of the free text and leaves every other word', () => {
  expect(takeStage('stage:done')).toEqual({ q: '', stage: 'done' });
  expect(takeStage('notes STAGE:Discussion beta')).toEqual({
    q: 'notes beta',
    stage: 'discussion',
  });
  expect(takeStage('stage:done stage:in-progress')).toEqual({ q: '', stage: 'in-progress' });
  expect(takeStage('stage:all')).toEqual({ q: '', stage: 'all' });
  // An unknown name is text, not a stage; no word is dropped without a reason.
  expect(takeStage('stage:blocked stage: stage')).toEqual({ q: 'stage:blocked stage: stage' });
  expect(takeStage('')).toEqual({ q: '' });
});

it('completes project:, by: and stage: before the caret, with the right start', () => {
  expect(filterQuery('stage:do', 8)).toEqual({ kind: 'stage', start: 0, end: 8, query: 'do' });
  expect(filterQuery('notes Stage:', 12)).toEqual({ kind: 'stage', start: 6, end: 12, query: '' });
  expect(filterQuery('by:ad', 5)).toEqual({ kind: 'by', start: 0, end: 5, query: 'ad' });
  expect(filterQuery('project:q4', 10)).toEqual({
    kind: 'project',
    start: 0,
    end: 10,
    query: 'q4',
  });
  expect(filterQuery('upstage:do', 10)).toBeUndefined();
});
