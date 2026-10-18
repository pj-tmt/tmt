import { expect, it } from 'vite-plus/test';
import { expiryText, localTime } from '../src/expiry.js';
const now = Date.UTC(2026, 9, 4, 12, 34, 56, 789);
const base = {
  retentionDays: 30,
  lastUpdateAtMs: now - 24 * 86400000,
  expiresAtMs: now + 6 * 86400000,
  warnings: ['expires-soon'],
};
it('names expiry with relative days and hours, as the CLI does', () => {
  expect(expiryText(base, now)).toBe('expires in 6 days');
  expect(expiryText({ ...base, expiresAtMs: now + 86400000 }, now)).toContain('in 1 day');
  expect(expiryText({ ...base, expiresAtMs: now + 5 * 3600000 }, now)).toContain('in 5 h');
  expect(expiryText({ ...base, expiresAtMs: now + 3600000 - 1 }, now)).toContain(
    'in less than an hour',
  );
  // Past the cloud expiry the cloud copy is gone, however long ago it ended.
  expect(
    expiryText({ ...base, expiresAtMs: now - 2 * 86400000, warnings: ['not-in-cloud'] }, now),
  ).toBe('not in the cloud');
  expect(expiryText({ ...base, expiresAtMs: now, warnings: ['not-in-cloud'] }, now)).toBe(
    'not in the cloud',
  );
  expect(expiryText(base, now)).not.toMatch(/\d{4}-|UTC|\.789/);
});
it('keeps local-only and forever evidence friendly without fabricating a relative date', () => {
  // Nothing was ever put in the cloud: no expiry to show and nothing to warn of.
  expect(expiryText({ ...base, lastUpdateAtMs: null, expiresAtMs: null, warnings: [] }, now)).toBe(
    'local only',
  );
  expect(expiryText({ ...base, expiresAtMs: null, warnings: ['not-in-cloud'] }, now)).toBe(
    'not in the cloud',
  );
  expect(expiryText({ ...base, retentionDays: null, expiresAtMs: null }, now)).toBe('kept forever');
  expect(
    expiryText(
      { ...base, retentionDays: null, expiresAtMs: null, warnings: ['not-in-cloud'] },
      now,
    ),
  ).toBe('kept forever');
});
it('names both facts for a locally forever page with a confirmed cloud copy', () => {
  const forever = { ...base, retentionDays: null };
  expect(expiryText(forever, now)).toBe('kept forever here · cloud copy expires in 6 days');
  expect(expiryText({ ...forever, expiresAtMs: now + 300 * 86400000, warnings: [] }, now)).toBe(
    'kept forever here · cloud copy expires in 300 days',
  );
  expect(expiryText({ ...forever, expiresAtMs: now, warnings: ['not-in-cloud'] }, now)).toBe(
    'kept forever here · not in the cloud',
  );
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
