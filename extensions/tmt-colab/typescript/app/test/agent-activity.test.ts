import { describe, expect, it } from 'vite-plus/test';
import {
  ActivityTracker,
  activityAt,
  agentActivity,
  type ActivityObservation,
} from '../src/agent-activity.js';

const operation = { operationId: 'operation', machine: 'machine', agent: 'agent' };
describe('agent activity is owner-clock presentation, never receipt authority', () => {
  it('retains only valid optional telemetry and never coerces a timestamp', () => {
    expect(agentActivity({ state: 'working', sinceMs: 10, secret: 'discard' })).toEqual({
      state: 'working',
      sinceMs: 10,
    });
    for (const value of [
      null,
      [],
      {},
      { state: 'busy', sinceMs: 10 },
      { state: 'idle', sinceMs: '10' },
      { state: 'working', sinceMs: -1 },
      { state: 'working', sinceMs: NaN },
    ])
      expect(agentActivity(value)).toBeUndefined();
    expect(agentActivity({ state: 'unknown', sinceMs: null })).toEqual({
      state: 'unknown',
      sinceMs: null,
    });
  });
  it('compares phase start only with a pre-Send owner anchor, including equality', () => {
    const tracker = new ActivityTracker();
    tracker.anchor(operation.operationId, 1_000);
    expect(
      tracker.read(operation, {
        activity: { state: 'working', sinceMs: 999 },
        observedAtMs: 2_000,
      }),
    ).toMatchObject({ kind: 'queued', elapsedMs: 1_001 });
    expect(
      tracker.read(operation, {
        activity: { state: 'working', sinceMs: 1_000 },
        observedAtMs: 2_000,
      }),
    ).toMatchObject({ kind: 'working', elapsedMs: 1_000 });
    expect(
      tracker.read(operation, { activity: { state: 'idle', sinceMs: 2_500 }, observedAtMs: 3_000 }),
    ).toMatchObject({ kind: 'finished', elapsedMs: 500 });
  });
  it('never guesses completion from idle, queued working, absent anchors or missing owner clock', () => {
    const tracker = new ActivityTracker();
    expect(
      tracker.read(operation, { activity: { state: 'working', sinceMs: 100 }, observedAtMs: 200 })
        .kind,
    ).toBe('sent');
    tracker.anchor(operation.operationId, 100);
    expect(
      tracker.read(operation, { activity: { state: 'idle', sinceMs: 100 }, observedAtMs: 200 })
        .kind,
    ).toBe('sent');
    expect(
      tracker.read(operation, { activity: { state: 'working', sinceMs: 99 }, observedAtMs: 200 })
        .kind,
    ).toBe('queued');
    expect(
      tracker.read(operation, { activity: { state: 'idle', sinceMs: 110 }, observedAtMs: 200 })
        .kind,
    ).toBe('sent');
    expect(tracker.read(operation, { activity: { state: 'working', sinceMs: 100 } }).kind).toBe(
      'sent',
    );
    expect(
      tracker.read(operation, { activity: { state: 'working', sinceMs: 300 }, observedAtMs: 200 })
        .kind,
    ).toBe('sent');
    expect(tracker.read(operation).kind).toBe('sent');
  });
  it('uses local monotonic read age only for elapsed and freshness, and stops animation', () => {
    const observation: ActivityObservation = {
      active: true,
      receivedAt: 100,
      intervalMs: 2_000,
      entries: [{ ...operation, kind: 'working', elapsedMs: 5_000 }],
    };
    expect(activityAt(observation, operation, 1_100)).toMatchObject({
      kind: 'working',
      elapsedMs: 6_000,
    });
    expect(activityAt(observation, operation, 4_100)?.kind).toBe('unknown');
    expect(activityAt({ ...observation, active: false }, operation, 1_100)?.kind).toBe('sent');
    expect(
      activityAt(
        {
          ...observation,
          entries: [{ ...operation, kind: 'finished', elapsedMs: 20 }],
          active: false,
        },
        operation,
        10_000,
      ),
    ).toMatchObject({ kind: 'finished', elapsedMs: 20 });
    expect(activityAt(observation, { ...operation, machine: 'other' }, 100)).toBeUndefined();
    expect(activityAt(observation, { ...operation, agent: 'other' }, 100)).toBeUndefined();
    expect(activityAt(observation, { ...operation, operationId: 'other' }, 100)).toBeUndefined();
  });
  it('a future hosted observation uses a 60-second interval and 120-second freshness limit', () => {
    const observation: ActivityObservation = {
      active: true,
      receivedAt: 100,
      intervalMs: 60000,
      entries: [{ ...operation, kind: 'working', elapsedMs: 5000 }],
    };
    expect(activityAt(observation, operation, 120099)?.kind).toBe('working');
    expect(activityAt(observation, operation, 120100)?.kind).toBe('unknown');
  });
});

it('keeps Sent when working is known but its phase clock cannot classify this turn', () => {
  const tracker = new ActivityTracker();
  tracker.anchor(operation.operationId, 1000);
  for (const telemetry of [
    { activity: { state: 'working' as const, sinceMs: null }, observedAtMs: 2000 },
    { activity: { state: 'working' as const, sinceMs: 3000 }, observedAtMs: 2000 },
    { activity: { state: 'working' as const, sinceMs: 1000 } },
  ])
    expect(tracker.read(operation, telemetry)).toMatchObject({ kind: 'sent', elapsedMs: 0 });
  for (const state of ['unknown', 'ended'] as const)
    expect(
      tracker.read(operation, { activity: { state, sinceMs: null }, observedAtMs: 2000 }).kind,
    ).toBe('unknown');
});
