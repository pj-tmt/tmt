import { expect, test } from 'claude-code/testing';

// The engine loads the mod from this folder with its own host, which can run no process: every
// push fails, and the mod must still answer each hook and never disturb the session.
const limits = [
  { kind: 'five_hour', percentUsed: 7, resetsAt: '2026-10-25T17:10:00.000Z' },
  { kind: 'seven_day', percentUsed: 59, resetsAt: '2026-10-28T08:00:00.000Z' },
];

test('a measure passes through when tmt cannot be run', async ($, on) => {
  on('session.measure', async (_$, e) => ({ changed: e.changed }));
  const result = await $.session.measure({
    context: {},
    rateLimits: limits,
    changed: ['rateLimits'],
  } as never);
  expect(result).toEqual({ changed: ['rateLimits'] });
});

test('a measure without rate limits passes through', async ($, on) => {
  on('session.measure', async (_$, e) => ({ changed: e.changed }));
  const result = await $.session.measure({
    context: {},
    rateLimits: [],
    changed: ['context'],
  } as never);
  expect(result).toEqual({ changed: ['context'] });
});
