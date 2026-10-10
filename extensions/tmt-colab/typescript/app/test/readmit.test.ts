import { expect, it } from 'vite-plus/test';
import { ReadmitBudget, readmittable } from '../src/readmit.js';

it.each(['STALE_EPOCH', 'DENIED', 'EXPIRED'])('%s on an open page is re-admittable', (code) => {
  expect(readmittable(new Error(code))).toBe(true);
});

it.each(['Page unavailable', 'CAPACITY', 'REMOTE_DEVICE_REVOKED', 'Access ended', 'GAP'])(
  '%s is not re-admitted automatically',
  (code) => {
    expect(readmittable(new Error(code))).toBe(false);
  },
);

it('allows three reopens per minute, then stops until the window passes', () => {
  const budget = new ReadmitBudget();
  expect([0, 1000, 2000].map((t) => budget.take(t))).toEqual([true, true, true]);
  expect(budget.take(3000)).toBe(false);
  expect(budget.take(59_999)).toBe(false);
  expect(budget.take(60_001)).toBe(true);
});
