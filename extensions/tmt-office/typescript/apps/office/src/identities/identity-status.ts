/** Read-only projection of the canonical identity status, not appearance or presence. */
export interface IdentityStatusSnapshot {
  activity: string;
  mood: string | null;
  updatedAtMs: number;
  expiresAtMs: number;
  stale: boolean;
}

export function decodeIdentityStatus(value: unknown): IdentityStatusSnapshot | null {
  if (value === null) return null;
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Invalid identity status.');
  const record = value as Record<string, unknown>;
  const text = (value: unknown, limit: number): value is string =>
    typeof value === 'string' &&
    // Match core's Unicode White_Space test, not ECMAScript trim (which adds BOM).
    /\P{White_Space}/u.test(value) &&
    new TextEncoder().encode(value).length <= limit &&
    !/\p{Cc}/u.test(value);
  if (
    Object.keys(record).sort().join(',') !== 'activity,expiresAtMs,mood,stale,updatedAtMs' ||
    !text(record.activity, 160) ||
    !(record.mood === null || text(record.mood, 32)) ||
    typeof record.stale !== 'boolean' ||
    !Number.isSafeInteger(record.updatedAtMs) ||
    !Number.isSafeInteger(record.expiresAtMs) ||
    (record.updatedAtMs as number) <= 0 ||
    (record.expiresAtMs as number) - (record.updatedAtMs as number) < 1_000 ||
    (record.expiresAtMs as number) - (record.updatedAtMs as number) > 86_400_000
  )
    throw new Error('Invalid identity status.');
  return record as unknown as IdentityStatusSnapshot;
}

/** Host-reported stale data cannot become fresh because a browser clock differs. */
export function isStatusFresh(status: IdentityStatusSnapshot | null, nowMs: number): boolean {
  return (
    status !== null && !status.stale && nowMs >= status.updatedAtMs && nowMs < status.expiresAtMs
  );
}

export function statusCue(
  status: IdentityStatusSnapshot | null,
  nowMs: number
): string | undefined {
  if (!status || !isStatusFresh(status, nowMs)) return undefined;
  return status.mood ? `${status.mood} · ${status.activity}` : status.activity;
}
