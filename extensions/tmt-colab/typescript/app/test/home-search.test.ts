import { expect, it } from 'vite-plus/test';
import { validateHomeSearch } from '../src/home.js';

it('names every key, so one it cannot read replaces the raw value the route merges under it', () => {
  const dropped = validateHomeSearch({
    label: 'design',
    by: { name: 'Stu' },
    q: 123,
    sort: 1,
    archived: 1,
    needs: 1,
    other: 'kept by the route, never read',
  });
  expect(Object.keys(dropped).sort()).toEqual(['archived', 'by', 'label', 'needs', 'q', 'sort']);
  expect(Object.values(dropped).every((value) => value === undefined)).toBe(true);
});

it('keeps what it can read and bounds it', () => {
  expect(
    validateHomeSearch({
      label: ['design', 7, 'design', 'q4 plan'],
      by: ['Stu', '', 'Ana\u0007'],
      q: '  notes  ',
      sort: 'title',
      archived: 'true',
      needs: true,
    }),
  ).toEqual({
    label: ['design', 'q4 plan'],
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
