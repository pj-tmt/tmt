import { expect, it } from 'vite-plus/test';
import { memberState } from '../src/ops-channel.js';

it('uses only explicit offline and activity for the four uncoloured channel states', () => {
  expect(memberState({ presence: 'offline', activity: { state: 'working' } })).toBe('Offline');
  expect(memberState({ presence: 'active', activity: { state: 'working' } })).toBe('Working');
  expect(memberState({ activity: { state: 'idle' } })).toBe('Idle');
  for (const state of ['ended', 'unknown', 'blocked', 'needs-you'])
    expect(memberState({ activity: { state } })).toBe('Unknown');
  expect(memberState({})).toBe('Unknown');
});
