import { expect, it } from 'vite-plus/test';
import { validateHomeSearch } from '../src/home.js';

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
