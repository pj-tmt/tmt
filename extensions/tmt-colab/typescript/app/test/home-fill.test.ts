import { describe, expect, it } from 'vite-plus/test';
import { REREADS, fillStep } from '../src/home-fill.js';

describe('fillStep', () => {
  it('stops when no page is missing a summary', () => {
    expect(fillStep(0, 0, true)).toBe('stop');
    expect(fillStep(0, 7, false)).toBe('stop');
  });

  it('schedules the next read while pages are missing and the tab is visible', () => {
    expect(fillStep(3, 0, true)).toBe('schedule');
    expect(fillStep(1, REREADS - 1, true)).toBe('schedule');
  });

  it('waits in a hidden tab and never reads there', () => {
    expect(fillStep(3, 0, false)).toBe('wait');
  });

  it('stops after the last allowed read, hidden or not', () => {
    expect(fillStep(3, REREADS, true)).toBe('stop');
    expect(fillStep(3, REREADS, false)).toBe('stop');
  });
});
