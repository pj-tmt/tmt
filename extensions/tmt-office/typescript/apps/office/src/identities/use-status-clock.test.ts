import { act, renderHook } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { useStatusClock } from './use-status-clock.js';
import { statusCue } from './identity-status.js';

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});
const status = {
  activity: 'Reviewing',
  mood: null,
  updatedAtMs: 1000,
  expiresAtMs: 2000,
  stale: false,
};

it('uses one nearest deadline across identities, expires without another read and cancels on unmount', () => {
  vi.useFakeTimers();
  vi.setSystemTime(1000);
  const profiles = [status, { ...status, expiresAtMs: 3000 }].map((selfReportedStatus) => ({
    selfReportedStatus,
  }));
  const { result, unmount } = renderHook(() => useStatusClock(profiles));
  expect(vi.getTimerCount()).toBe(1);
  act(() => vi.advanceTimersByTime(999));
  expect(statusCue(status, result.current)).toBe('Reviewing');
  act(() => vi.advanceTimersByTime(1));
  expect(statusCue(status, result.current)).toBeUndefined();
  expect(statusCue(profiles[1]!.selfReportedStatus, result.current)).toBe('Reviewing');
  expect(vi.getTimerCount()).toBe(1);
  act(() => vi.advanceTimersByTime(1000));
  expect(statusCue(profiles[1]!.selfReportedStatus, result.current)).toBeUndefined();
  expect(vi.getTimerCount()).toBe(0);
  unmount();
});

it('resamples on renewal, visibility and clock rollback without leaving a timer after removal', () => {
  vi.useFakeTimers();
  vi.setSystemTime(1000);
  const { result, rerender, unmount } = renderHook(({ profiles }) => useStatusClock(profiles), {
    initialProps: { profiles: [{ selfReportedStatus: status }] },
  });
  const renewed = { ...status, updatedAtMs: 1500, expiresAtMs: 4000 };
  vi.setSystemTime(1500);
  rerender({ profiles: [{ selfReportedStatus: renewed }] });
  act(() => vi.advanceTimersByTime(500));
  expect(statusCue(renewed, result.current)).toBe('Reviewing');
  vi.setSystemTime(1400);
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible');
  act(() => document.dispatchEvent(new Event('visibilitychange')));
  expect(statusCue(renewed, result.current)).toBeUndefined();
  expect(vi.getTimerCount()).toBe(0);
  vi.setSystemTime(3000);
  act(() => document.dispatchEvent(new Event('visibilitychange')));
  expect(statusCue(renewed, result.current)).toBe('Reviewing');
  expect(vi.getTimerCount()).toBe(1);
  rerender({ profiles: [] });
  expect(vi.getTimerCount()).toBe(0);
  rerender({ profiles: [{ selfReportedStatus: renewed }] });
  expect(vi.getTimerCount()).toBe(1);
  unmount();
  expect(vi.getTimerCount()).toBe(0);
});
