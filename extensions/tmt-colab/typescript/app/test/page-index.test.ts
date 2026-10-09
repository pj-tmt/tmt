import { expect, it } from 'vite-plus/test';
import { orderPages, pageTitle, pageUpdate } from '../src/page-index.js';
import type { PageSummary } from '../src/transport.js';

it('orders newest first, then displayed English title and ID, without changing the input', () => {
  const pages: readonly PageSummary[] = Object.freeze([
    { id: 'old', title: 'Older', sharing: 'private', lastUpdateAtMs: 100 },
    { id: 'beta', title: 'Beta', sharing: 'public', lastUpdateAtMs: 200 },
    { id: 'alpha-b', title: ' Alpha ', sharing: 'link', lastUpdateAtMs: 200 },
    { id: 'alpha-a', title: 'Alpha', sharing: 'private', lastUpdateAtMs: 200 },
    { id: 'unknown', title: 'Zebra', sharing: 'private', lastUpdateAtMs: null },
    { id: 'missing', title: 'Aardvark', sharing: 'private' },
    { id: 'negative', title: 'Before epoch', sharing: 'private', lastUpdateAtMs: -100 },
    { id: 'zero', title: 'Epoch', sharing: 'private', lastUpdateAtMs: 0 },
  ]);
  const original = [...pages];
  expect(orderPages(pages).map((p) => p.id)).toEqual([
    'alpha-a',
    'alpha-b',
    'beta',
    'old',
    'zero',
    'negative',
    'missing',
    'unknown',
  ]);
  expect(pages).toEqual(original);
  expect(pageTitle({ id: 'empty', title: ' ', sharing: 'private' })).toBe('Untitled page');
});

it('uses one render time for relative labels and exact ISO dates without inventing unknown dates', () => {
  const now = Date.UTC(2026, 9, 9);
  const updated = now - 3600000;
  expect(pageUpdate(updated, now)).toMatchObject({
    label: '1 hour ago',
    dateTime: '2026-10-08T23:00:00.000Z',
  });
  expect(pageUpdate(updated, now).absolute).toMatch(/2026/);
  for (const value of [null, undefined, Number.MAX_SAFE_INTEGER]) {
    expect(pageUpdate(value, now)).toEqual({ label: 'Update time unknown' });
  }
});
