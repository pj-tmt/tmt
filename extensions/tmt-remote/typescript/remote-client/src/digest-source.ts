/**
 * What the Digest web view (`/digest/`) reads: the usage report the terminal Digest shows. The
 * types follow the public document of `tmt digest usage report --json` (digest `usage-v1.md`) and
 * the exchange flow of `tmt x flow --json`; the page depends only on `DigestSource`. The product
 * source reads the owner's browser reads (`x/-/digest/<range>` and `x/-/limits`, contracts/
 * remote-channel-v1.md); the seeded sample lives in `digest-mock.ts` for tests, the gallery and
 * dev only.
 */
import { ownerRead, type OwnerReadAnswer, type OwnerReadReason } from './owner-reads.js';

export const USAGE_RANGES = ['24h', '7d', '30d'] as const;
export type UsageRange = (typeof USAGE_RANGES)[number];

export interface UsageTotals {
  input: number;
  output: number;
  /** Part of `input`, never added to it. */
  cached: number;
}

export interface UsagePoint extends UsageTotals {
  fromMs: number;
  toMs: number;
  /** False: the bucket is a lower bound, not an error. */
  complete: boolean;
}

export interface UsageSeat {
  id: string;
  /** Null when Core could not resolve the name. */
  name: string | null;
  retired: boolean;
  totals: UsageTotals;
  /** The model families its tokens belong to (`claude`, `gpt`, `other`, `unattributed`). */
  lines: readonly string[];
  /** Hourly for 24h, daily for 7d and 30d, oldest first. */
  series: readonly UsagePoint[];
  coverage: {
    coveredMs: number;
    windowMs: number;
    startsAtMs: number | null;
    complete: boolean;
    gap: boolean;
  };
}

export interface UsageUnavailable {
  id: string;
  name: string | null;
  retired: boolean;
  reason: string;
}

/** An account window, newest observation wins; windows are never summed. */
export interface LimitWindow {
  /** The driver of the source line, as Core reports it; never inferred from a seat's models. */
  provider: string;
  windowMinutes: number;
  resetsAtMs: number;
  usedPercent: number;
  observedAtMs: number;
  /** Null for a stale window. */
  elapsedPercent: number | null;
  paceDeltaPoints: number | null;
  stale: boolean;
  /** The seats that reported it. */
  seats: readonly string[];
}

export interface FlowParty {
  id: string | null;
  name: string | null;
  retired: boolean;
}

export interface FlowPair {
  sender: FlowParty;
  recipient: FlowParty;
  /** `talk`, `reply`, `ack`, `result`; the page counts talk and reply like the terminal. */
  kind: string;
  count: number;
}

/**
 * Percent left of a weekly window sampled over time, from `x/-/limits` (the board's local cache).
 * Without it the page shows the latest window only.
 */
export interface LimitHistory {
  provider: string;
  windowMinutes: number;
  resetsAtMs: number;
  points: readonly { atMs: number; percentLeft: number }[];
}

export interface UsageDocument {
  version: 1;
  observedAtMs: number;
  since: UsageRange;
  usage: {
    seats: readonly UsageSeat[];
    unavailable: readonly UsageUnavailable[];
  };
  limits: readonly LimitWindow[];
  limitsUnavailable: readonly { line: string; reason: string }[];
  exchange: { sinceMs: number; observedAtMs: number; pairs: readonly FlowPair[] } | null;
  limitHistory?: readonly LimitHistory[];
}

export interface AllowanceReport {
  /** Fixed weekly snapshot, independent of the selected usage range. */
  allowance?: UsageDocument;
  allowanceUnavailable?: string;
}

export interface DigestReport extends AllowanceReport {
  range: UsageRange;
  /** Whether this is sample data, not the owner's usage. */
  sample: boolean;
  /** Why the source cannot report; the page then shows only this. */
  unavailable?: string;
  document?: UsageDocument;
}

export interface DigestSource {
  /** Range switches reuse this mount's allowance; the minute refresh requests a fresh read. */
  report(
    range: UsageRange,
    signal?: AbortSignal,
    refreshAllowance?: boolean,
  ): Promise<DigestReport>;
}

/** What `x/-/digest/<range>` answers: the usage report document, unchanged, under `report`. */
interface DigestPayload {
  since: UsageRange;
  report: UsageDocument;
}

/** What `x/-/limits` answers: the weekly samples the board keeps, as the terminal prints them. */
interface LimitsPayload {
  limitHistory: readonly {
    provider: string;
    windowMinutes: number;
    /** `[observedAtMs, percentLeft, resetsAtMs]`, oldest first. */
    points: readonly (readonly [number, number, number])[];
  }[];
}

/** One plain sentence per reason, each saying what the person can do. */
const REASONS: Record<OwnerReadReason | 'no-session' | 'unreachable', string> = {
  not_installed:
    'Digest is not installed on your machine. Run `tmt extension install digest`, then reload this page.',
  core_unavailable: 'TMT on your machine did not answer. This page tries again shortly.',
  timeout: 'Your machine took too long to read usage. This page tries again shortly.',
  read_failed:
    'Your machine could not read usage. Run `tmt digest usage report` in a terminal to see it.',
  too_large:
    'The usage report is too large for the browser. Run `tmt digest usage report` in a terminal to see it.',
  not_sampled:
    'Your machine could not read usage. Run `tmt digest usage report` in a terminal to see it.',
  unknown:
    'Usage is not available right now. Run `tmt digest usage report` in a terminal to see it.',
  'no-session':
    'This browser has no owner session for your machine. Pair it from Remote to see your usage.',
  unreachable: 'Your machine did not answer. This page tries again shortly.',
};

/** Keep each reset's samples separate, including older windows still active this week. */
export function limitHistoryFrom(
  answer: OwnerReadAnswer<LimitsPayload>,
): readonly LimitHistory[] | undefined {
  if (answer.state !== 'ok') return undefined;
  const histories: unknown = answer.payload.limitHistory;
  if (!Array.isArray(histories)) return undefined;
  const isPoint = (point: unknown): point is readonly [number, number, number] =>
    Array.isArray(point) && point.length === 3 && point.every((value) => Number.isFinite(value));
  return histories.flatMap((entry: unknown) => {
    const history = entry as Partial<LimitsPayload['limitHistory'][number]> | null;
    if (
      typeof history?.provider !== 'string' ||
      !Number.isFinite(history.windowMinutes) ||
      !Array.isArray(history.points)
    )
      return [];
    const groups = new Map<number, { atMs: number; percentLeft: number }[]>();
    for (const [atMs, percentLeft, reset] of history.points.filter(isPoint)) {
      const points = groups.get(reset) ?? [];
      points.push({ atMs, percentLeft });
      groups.set(reset, points);
    }
    return [...groups].map(([resetsAtMs, points]) => ({
      provider: history.provider!,
      windowMinutes: history.windowMinutes as number,
      resetsAtMs,
      points: points.sort((a, b) => a.atMs - b.atMs),
    }));
  });
}

/** A failed weekly read stays explicit; the selected usage range is never its fallback. */
export function allowanceFromAnswers(
  weekly: OwnerReadAnswer<DigestPayload>,
  limits: OwnerReadAnswer<LimitsPayload>,
): AllowanceReport {
  if (
    weekly.state !== 'ok' ||
    weekly.payload.since !== '7d' ||
    weekly.payload.report?.since !== '7d'
  )
    return { allowanceUnavailable: 'Allowance could not be read. This page tries again shortly.' };
  const limitHistory = limitHistoryFrom(limits);
  return { allowance: { ...weekly.payload.report, ...(limitHistory ? { limitHistory } : {}) } };
}

/** Selected usage and independently acquired allowance through the existing owner reads. */
export function reportFromAnswers(
  range: UsageRange,
  digest: OwnerReadAnswer<DigestPayload>,
  allowance: AllowanceReport,
): DigestReport {
  if (digest.state !== 'ok') {
    const reason = digest.state === 'unavailable' ? digest.reason : digest.state;
    return { range, sample: false, ...allowance, unavailable: REASONS[reason] };
  }
  const document = digest.payload.report;
  if (digest.payload.since !== range || document?.since !== range)
    return { range, sample: false, ...allowance, unavailable: REASONS.unknown };
  return { range, sample: false, document, ...allowance };
}

/** One source per mount: range switches share even an in-flight weekly read, never global state. */
export function createDigestSource(): DigestSource {
  let weekly: Promise<OwnerReadAnswer<DigestPayload>> | undefined;
  let allowance: Promise<AllowanceReport> | undefined;
  return {
    async report(range, signal = new AbortController().signal, refreshAllowance = false) {
      if (refreshAllowance) {
        weekly = undefined;
        allowance = undefined;
      }
      const selected =
        range === '7d' && weekly ? weekly : ownerRead<DigestPayload>(`digest/${range}`, signal);
      if (!allowance) {
        weekly = range === '7d' ? selected : ownerRead<DigestPayload>('digest/7d', signal);
        allowance = Promise.all([weekly, ownerRead<LimitsPayload>('limits', signal)]).then(
          ([weekly, limits]) => allowanceFromAnswers(weekly, limits),
        );
      }
      const [digest, snapshot] = await Promise.all([selected, allowance]);
      return reportFromAnswers(range, digest, snapshot);
    },
  };
}
