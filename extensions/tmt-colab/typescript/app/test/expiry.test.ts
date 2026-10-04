import { expect, it } from 'vite-plus/test';
import { expiryText, utcTime } from '../src/expiry.js';
it('shows local expiry dates, warnings and friendly unknown evidence without revocation claims', () => {
  const base = { retentionDays: 30, lastUpdateAtMs: 0, expiresAtMs: 2592000000, warnings: [] };
  expect(expiryText(base)).toContain('1970-01-31 00:00:00.000 UTC');
  expect(expiryText({ ...base, warnings: ['expires-soon'] })).toContain('within seven days');
  expect(expiryText({ ...base, warnings: ['expired'] })).toContain('this page is still available');
  expect(
    expiryText({
      ...base,
      lastUpdateAtMs: null,
      expiresAtMs: null,
      warnings: ['expiry-unavailable'],
    }),
  ).toBe('Expiry starts after the next edit.');
  expect(expiryText({ ...base, retentionDays: null, expiresAtMs: null })).toBe(
    'No expiry: kept forever.',
  );
  expect(expiryText({ ...base, expiresAtMs: null, warnings: ['expiry-out-of-range'] })).toContain(
    'beyond the supported range',
  );
  expect(utcTime(951827696789)).toBe('2000-02-29 12:34:56.789 UTC');
  expect(utcTime(null)).toBe('Not recorded yet');
  expect(utcTime(Number.MAX_SAFE_INTEGER)).toContain('beyond the supported range');
});
