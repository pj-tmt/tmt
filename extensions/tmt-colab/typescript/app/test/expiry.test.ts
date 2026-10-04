import { expect, it } from 'vite-plus/test';
import { expiryText, localTime } from '../src/expiry.js';
const now = Date.UTC(2026, 9, 4, 12, 34, 56, 789);
const base = {
  retentionDays: 30,
  lastUpdateAtMs: now - 24 * 86400000,
  expiresAtMs: now + 6 * 86400000,
  warnings: ['expires-soon'],
};
it('names advisory retention with relative days and hours and preserves local copies', () => {
  expect(expiryText(base, now)).toBe('Retention ends in 6 days · advisory; local copy stays.');
  expect(expiryText({ ...base, expiresAtMs: now + 86400000 }, now)).toContain('in 1 day');
  expect(expiryText({ ...base, expiresAtMs: now + 5 * 3600000 }, now)).toContain('in 5 h');
  expect(expiryText({ ...base, expiresAtMs: now + 3600000 - 1 }, now)).toContain(
    'in less than an hour',
  );
  expect(expiryText({ ...base, expiresAtMs: now - 2 * 86400000, warnings: ['expired'] }, now)).toBe(
    'Retention ended 2 days ago · advisory; local copy stays.',
  );
  expect(expiryText({ ...base, expiresAtMs: now - 5 * 3600000 }, now)).toContain('ended 5 h ago');
  expect(expiryText(base, now)).not.toMatch(/\d{4}-|UTC|\.789/);
});
it('keeps unknown and forever evidence friendly without fabricating a relative date', () => {
  expect(
    expiryText(
      { ...base, lastUpdateAtMs: null, expiresAtMs: null, warnings: ['expiry-unavailable'] },
      now,
    ),
  ).toBe('Expiry starts after the next edit.');
  expect(expiryText({ ...base, retentionDays: null, expiresAtMs: null }, now)).toBe(
    'Kept forever.',
  );
  expect(
    expiryText({ ...base, expiresAtMs: null, warnings: ['expiry-out-of-range'] }, now),
  ).toContain('beyond the supported range');
  expect(localTime(null)).toBe('Not recorded yet');
  expect(localTime(Number.MAX_SAFE_INTEGER)).toContain('beyond the supported range');
});
it('formats the hover date in the local timezone, without seconds or milliseconds', () => {
  const date = new Date(base.expiresAtMs);
  const time = localTime(base.expiresAtMs);
  expect(time).toMatch(/^[A-Z][a-z]{2} \d{2}-\d{2} \d{2}:\d{2}$/);
  expect(
    time.endsWith(
      `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`,
    ),
  ).toBe(true);
});
