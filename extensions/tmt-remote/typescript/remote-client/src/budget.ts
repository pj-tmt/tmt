/**
 * Free-plan (Spark) Firestore budget guard, the browser twin of the Rust `firestore_budget`
 * module (#2180). Pure arithmetic: no I/O and no clock (callers pass `nowMs`). Quotas are
 * project-wide, every client sees only its own traffic and Rules cannot read quotas, so the
 * guard works from what a client really has: the member count from the signed membership, the
 * page's parameters, its own persisted counter for today and these constants. It does not read
 * Google's counter. Both implementations are tested against the same independent vectors, so
 * a change to either one fails until both agree; the dated numbers and the derivation are in
 * the Remote reference guide's free-plan table.
 */

/** Spark (no-cost) Firestore document reads per day. */
export const READS_PER_DAY = 50_000;
/** Spark (no-cost) Firestore document writes per day. */
export const WRITES_PER_DAY = 20_000;
/** Rules dependent documents read when a client reads: member projection or link enrollment, plus the page head. */
export const READ_LOOKUPS = 2;
/** Dependent documents when a client appends: the same two plus the head read for the create-only sequence. */
export const WRITE_LOOKUPS = 3;
/** The guard warns at this share of a client's modeled allowance and refuses at the second one. */
export const WARN_PERCENT = 70;
export const REFUSE_PERCENT = 90;
/** Planning assumption for the minimum flush interval: this many active seconds per day. */
export const ACTIVE_SECONDS_PER_DAY = 2 * 60 * 60;
/** Largest member count the model accepts. */
export const MAX_MEMBERS = 10_000;
/** The error code of a refusal made before any effect. */
export const BUDGET_EXHAUSTED = 'REMOTE_BUDGET_EXHAUSTED';

export type BudgetModelErrorCode = 'out_of_range' | 'free_plan_cannot_host';
/**
 * `out_of_range`: no members, no writers, more writers than members, or too many members.
 * `free_plan_cannot_host`: the modeled share is too small for the guard to allow even one
 * append. A page like that is refused when the model is built, never as a pause that would end
 * at the next reset and begin again.
 */
export class BudgetModelError extends Error {
  constructor(readonly code: BudgetModelErrorCode) {
    super(
      code === 'out_of_range'
        ? 'The page size is out of range.'
        : 'The free plan cannot host a page this size.',
    );
    this.name = 'BudgetModelError';
  }
}
export type BudgetVerdict = 'ok' | 'warn' | 'refuse';

/** One shared page: `members` have it open (every one a listener), `writers` of them append. */
export class BudgetModel {
  private constructor(
    readonly members: number,
    readonly writers: number,
  ) {}
  static create(members: number, writers: number): BudgetModel {
    const whole = (value: number) => Number.isSafeInteger(value);
    if (
      !(
        whole(members) &&
        whole(writers) &&
        members >= 1 &&
        members <= MAX_MEMBERS &&
        writers >= 1 &&
        writers <= members
      )
    )
      throw new BudgetModelError('out_of_range');
    const model = new BudgetModel(members, writers);
    if (model.refuseAt() === 0) throw new BudgetModelError('free_plan_cannot_host');
    return model;
  }
  /** Reads one append costs the project, pessimistically: the writer's lookups, then each other listener's. */
  readsPerAppend(): number {
    return WRITE_LOOKUPS + (this.members - 1) * (1 + READ_LOOKUPS);
  }
  /** Appends per day the free plan allows this page if it were the project's only traffic. */
  dailyAppendLimit(): number {
    return Math.min(Math.floor(READS_PER_DAY / this.readsPerAppend()), WRITES_PER_DAY);
  }
  /** One client's modeled share of the page's daily allowance. */
  share(): number {
    return Math.floor(this.dailyAppendLimit() / this.writers);
  }
  /** Appends after which the guard warns, and the count at which it refuses. */
  warnAt(): number {
    return Math.floor((this.share() * WARN_PERCENT) / 100);
  }
  refuseAt(): number {
    return Math.floor((this.share() * REFUSE_PERCENT) / 100);
  }
  /** Shortest average interval between flushed updates, in milliseconds, that keeps the planning assumption. */
  minFlushIntervalMs(): number {
    const appends = Math.max(Math.floor((this.dailyAppendLimit() * WARN_PERCENT) / 100), 1);
    return Math.ceil((ACTIVE_SECONDS_PER_DAY * 1000) / appends);
  }
  /** What the guard says about the next append, given how many this client sent today. */
  assess(appendsToday: number): BudgetVerdict {
    if (appendsToday >= this.refuseAt()) return 'refuse';
    if (appendsToday >= this.warnAt()) return 'warn';
    return 'ok';
  }
}

const DAY_MS = 86_400_000;
const HOUR_MS = 3_600_000;
const floorDiv = (value: number, divisor: number) => Math.floor(value / divisor);
/** Days since 1970-01-01 of a proleptic Gregorian date. */
function daysFromCivil(year: number, month: number, day: number): number {
  const y = month <= 2 ? year - 1 : year;
  const era = floorDiv(y, 400);
  const yoe = y - era * 400;
  const mp = (month + 9) % 12;
  const doy = floorDiv(153 * mp + 2, 5) + day - 1;
  const doe = yoe * 365 + floorDiv(yoe, 4) - floorDiv(yoe, 100) + doy;
  return era * 146_097 + doe - 719_468;
}
function civilYear(days: number): number {
  const z = days + 719_468;
  const era = floorDiv(z, 146_097);
  const doe = z - era * 146_097;
  const yoe = floorDiv(
    doe - floorDiv(doe, 1460) + floorDiv(doe, 36_524) - floorDiv(doe, 146_096),
    365,
  );
  const doy = doe - (365 * yoe + floorDiv(yoe, 4) - floorDiv(yoe, 100));
  const mp = floorDiv(5 * doy + 2, 153);
  const month = mp < 10 ? mp + 3 : mp - 9;
  return yoe + era * 400 + (month <= 2 ? 1 : 0);
}
/** The n-th Sunday (1-based) of `month` in `year`, as a day number. */
function nthSunday(year: number, month: number, n: number): number {
  const first = daysFromCivil(year, month, 1);
  // 1970-01-01 was a Thursday: day 4 counting Sunday as 0, so (first + 4) mod 7 is the weekday.
  const weekday = (((first + 4) % 7) + 7) % 7;
  return first + ((7 - weekday) % 7) + (n - 1) * 7;
}
/** United States rule since 2007: daylight time from 02:00 local on the second Sunday of March to 02:00 local on the first Sunday of November. */
function isDstLocal(localMs: number): boolean {
  const year = civilYear(floorDiv(localMs, DAY_MS));
  const start = nthSunday(year, 3, 2) * DAY_MS + 2 * HOUR_MS;
  const end = nthSunday(year, 11, 1) * DAY_MS + 2 * HOUR_MS;
  return localMs >= start && localMs < end;
}
/** The Pacific UTC offset at a UTC instant: -8 h, or -7 h in daylight time. */
function offsetMs(utcMs: number): number {
  const year = civilYear(floorDiv(utcMs - 8 * HOUR_MS, DAY_MS));
  const start = nthSunday(year, 3, 2) * DAY_MS + 10 * HOUR_MS;
  const end = nthSunday(year, 11, 1) * DAY_MS + 9 * HOUR_MS;
  return utcMs >= start && utcMs < end ? -7 * HOUR_MS : -8 * HOUR_MS;
}
/** A Pacific calendar day (days since 1970-01-01 of the Pacific local date) and the instant its quotas next reset. */
export interface PacificDay {
  day: number;
  resetAtMs: number;
}
export function pacificDay(nowMs: number): PacificDay {
  const day = floorDiv(nowMs + offsetMs(nowMs), DAY_MS);
  const nextMidnightLocal = (day + 1) * DAY_MS;
  const nextOffset = isDstLocal(nextMidnightLocal) ? -7 * HOUR_MS : -8 * HOUR_MS;
  return { day, resetAtMs: nextMidnightLocal - nextOffset };
}

/** A client's appends today, rolled over when the Pacific day changes. */
export interface BudgetUsage {
  day: number;
  appends: number;
}
export function newUsage(nowMs: number): BudgetUsage {
  return { day: pacificDay(nowMs).day, appends: 0 };
}
/** The same usage if still the same Pacific day, else a fresh day. */
export function usageAt(usage: BudgetUsage, nowMs: number): BudgetUsage {
  return pacificDay(nowMs).day === usage.day ? usage : newUsage(nowMs);
}
/** Count one append sent. */
export function recordedUsage(usage: BudgetUsage, nowMs: number): BudgetUsage {
  const today = usageAt(usage, nowMs);
  return { ...today, appends: Math.min(today.appends + 1, Number.MAX_SAFE_INTEGER) };
}

/**
 * An append refused by the guard BEFORE any effect: nothing was sent, so the same sealed update
 * can be sent unchanged after the reset. Carries when.
 */
export interface BudgetRefusal {
  resetAtMs: number;
  retryAfterMs: number;
}
export type BudgetDecision =
  | { decision: 'allow' }
  /** Allowed, and the client should tell its user the share is nearly spent. */
  | { decision: 'warn' }
  | ({ decision: 'refuse' } & BudgetRefusal);
/** Judge the next append. Rolls the usage over to today first. */
export function decide(model: BudgetModel, usage: BudgetUsage, nowMs: number): BudgetDecision {
  const today = usageAt(usage, nowMs);
  const verdict = model.assess(today.appends);
  if (verdict === 'ok') return { decision: 'allow' };
  if (verdict === 'warn') return { decision: 'warn' };
  const { resetAtMs } = pacificDay(nowMs);
  return { decision: 'refuse', resetAtMs, retryAfterMs: Math.max(resetAtMs - nowMs, 0) };
}

/**
 * Whether a provider `resource-exhausted` answer is a refusal or an unknown outcome depends only
 * on whether the request could have changed anything. A read cannot have: it is reported as
 * `REMOTE_BUDGET_EXHAUSTED` with the reset. A write may have been applied before the answer: an
 * unknown outcome, never that code; the caller reads back the original ID and does not resend.
 */
export type ExhaustedOutcome = ({ outcome: 'refused' } & BudgetRefusal) | { outcome: 'unknown' };
/** A recorded provider refusal; it says nothing after the quota resets. */
export interface ExhaustedEvidence {
  day: number;
  resetAtMs: number;
}
export function classifyProviderExhausted(
  attempt: { mutation: boolean },
  nowMs: number,
): { outcome: ExhaustedOutcome; evidence: ExhaustedEvidence } {
  const { day, resetAtMs } = pacificDay(nowMs);
  const evidence = { day, resetAtMs };
  if (attempt.mutation) return { outcome: { outcome: 'unknown' }, evidence };
  return {
    outcome: { outcome: 'refused', resetAtMs, retryAfterMs: Math.max(resetAtMs - nowMs, 0) },
    evidence,
  };
}
/** Still a statement about today's quota. */
export function exhaustedIsCurrent(evidence: ExhaustedEvidence, nowMs: number): boolean {
  return nowMs < evidence.resetAtMs;
}
