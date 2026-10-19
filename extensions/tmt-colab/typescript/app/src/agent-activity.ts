/** Owner-clock telemetry is presentation only; it never proves delivery or grants a Send. */
export interface AgentActivity {
  state: 'working' | 'idle' | 'unknown' | 'ended';
  sinceMs: number | null;
}
export function agentActivity(value: unknown): AgentActivity | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return;
  const row = value as Record<string, unknown>;
  if (
    typeof row.state !== 'string' ||
    !['working', 'idle', 'unknown', 'ended'].includes(row.state) ||
    !(row.sinceMs === null || (Number.isSafeInteger(row.sinceMs) && Number(row.sinceMs) >= 0))
  )
    return;
  return { state: row.state as AgentActivity['state'], sinceMs: row.sinceMs as number | null };
}
export interface ActivityEntry {
  operationId: string;
  machine: string;
  agent: string;
  kind: 'sent' | 'working' | 'queued' | 'finished' | 'unknown';
  /** Owner-clock duration at acquisition, never compared with a viewer timestamp. */
  elapsedMs: number;
}
/** Agent-wide telemetry from the same read as the per-Ask lines; never an admission source. */
export interface ActivityAgent {
  machine: string;
  machineName: string;
  agent: string;
  name: string;
  state: AgentActivity['state'];
}
export interface ActivityObservation {
  active: boolean;
  receivedAt: number;
  intervalMs: number;
  entries: ActivityEntry[];
  agents?: ActivityAgent[];
}
/** An anchor and observed transition live only in this page's controller incarnation. */
export class ActivityTracker {
  #turns = new Map<string, { anchor: number; working: boolean; finished?: number }>();
  anchor(operationId: string, ownerTime: number | undefined): void {
    if (ownerTime !== undefined && Number.isSafeInteger(ownerTime) && ownerTime >= 0)
      this.#turns.set(operationId, { anchor: ownerTime, working: false });
  }
  read(
    operation: { operationId: string; machine: string; agent: string },
    telemetry?: { activity?: AgentActivity; observedAtMs?: number },
  ): ActivityEntry {
    const result: ActivityEntry = { ...operation, kind: 'sent', elapsedMs: 0 };
    const turn = this.#turns.get(operation.operationId);
    const activity = telemetry?.activity;
    const observed = telemetry?.observedAtMs;
    if (!turn || !activity || observed === undefined) return result;
    const since = activity.sinceMs;
    if (
      since === null ||
      since > observed ||
      activity.state === 'unknown' ||
      activity.state === 'ended'
    )
      return { ...result, kind: 'unknown' };
    if (activity.state === 'working') {
      if (since >= turn.anchor) turn.working = true;
      turn.finished = undefined;
      return {
        ...result,
        kind: since >= turn.anchor ? 'working' : 'queued',
        elapsedMs: observed - since,
      };
    }
    if (turn.working && since >= turn.anchor) turn.finished = since;
    return turn.finished === undefined
      ? result
      : { ...result, kind: 'finished', elapsedMs: observed - turn.finished };
  }
}

export function activityAt(
  observation: ActivityObservation | undefined,
  operation: { operationId: string; machine: string; agent: string },
  now: number,
): ActivityEntry | undefined {
  const row = observation?.entries.find(
    (entry) =>
      entry.operationId === operation.operationId &&
      entry.machine === operation.machine &&
      entry.agent === operation.agent,
  );
  if (!observation || !row) return;
  const age = Math.max(0, now - observation.receivedAt);
  if (!observation.active)
    return { ...row, kind: row.kind === 'working' || row.kind === 'queued' ? 'sent' : row.kind };
  if (age >= 2 * observation.intervalMs) return { ...row, kind: 'unknown' };
  return { ...row, elapsedMs: row.elapsedMs + age };
}
