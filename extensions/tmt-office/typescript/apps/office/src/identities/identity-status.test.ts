import { expect, it } from 'vitest';
import { decodeIdentityStatus, isStatusFresh, statusCue } from './identity-status.js';

const status = {
  activity: 'Reviewing',
  mood: 'focused',
  updatedAtMs: 1000,
  expiresAtMs: 2000,
  stale: false,
};

it('requires the complete canonical projection and byte, control and timestamp limits', () => {
  expect(decodeIdentityStatus(null)).toBeNull();
  expect(decodeIdentityStatus(status)).toEqual(status);
  expect(decodeIdentityStatus({ ...status, activity: '\uFEFF', mood: ' \uFEFF ' })).toBeDefined();
  expect(
    decodeIdentityStatus({ ...status, activity: 'a'.repeat(160), mood: 'm'.repeat(32) })
  ).toBeDefined();
  expect(
    decodeIdentityStatus({
      ...status,
      updatedAtMs: Number.MAX_SAFE_INTEGER - 1000,
      expiresAtMs: Number.MAX_SAFE_INTEGER,
    })
  ).toBeDefined();
  for (const value of [
    undefined,
    {},
    [],
    { ...status, unexpected: true },
    { ...status, activity: '😀'.repeat(41) },
    { ...status, mood: '😀'.repeat(9) },
    { ...status, mood: undefined },
    { ...status, activity: ' \t' },
    { ...status, activity: '\u2003\u3000' },
    { ...status, activity: 'Review\n' },
    { ...status, mood: '\u007f' },
    { ...status, updatedAtMs: 0 },
    { ...status, expiresAtMs: 1999 },
    { ...status, expiresAtMs: 86_401_001 },
    { ...status, expiresAtMs: Number.MAX_SAFE_INTEGER + 1 },
    { ...status, stale: 1 },
  ])
    expect(() => decodeIdentityStatus(value)).toThrow('Invalid identity status.');
});

it('expires at the exact deadline and fails closed for clock rollback or host-stale observations', () => {
  expect(isStatusFresh(status, 1000)).toBe(true);
  expect(statusCue(status, 1999)).toBe('focused · Reviewing');
  expect(statusCue({ ...status, mood: null }, 1000)).toBe('Reviewing');
  for (const now of [999, 2000, 2001]) expect(statusCue(status, now)).toBeUndefined();
  expect(statusCue({ ...status, stale: true }, 1500)).toBeUndefined();
  expect(statusCue(null, 1500)).toBeUndefined();
});
