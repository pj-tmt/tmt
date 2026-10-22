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
  line: string;
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

export interface DigestReport {
  range: UsageRange;
  /** Whether this is sample data, not the owner's usage. */
  sample: boolean;
  /** Why the source cannot report; the page then shows only this. */
  unavailable?: string;
  document?: UsageDocument;
}

export interface DigestSource {
  report(range: UsageRange, signal?: AbortSignal): Promise<DigestReport>;
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

/** The model family a driver's weekly window belongs to, as the report names it on its seats. */
const DRIVER_LINES: Record<string, string> = { claude: 'claude', codex: 'gpt' };

/**
 * The percent-left history of the current weekly window per provider. The cache keeps 8 days, so
 * its points can span two windows: only those of the newest window (its reset time) are drawn.
 */
export function limitHistoryFrom(
  answer: OwnerReadAnswer<LimitsPayload>,
): readonly LimitHistory[] | undefined {
  if (answer.state !== 'ok') return undefined;
  const histories: LimitsPayload['limitHistory'] | undefined = answer.payload.limitHistory;
  if (!Array.isArray(histories)) return undefined;
  return histories.flatMap((history) => {
    const last = history.points.at(-1);
    if (!last) return [];
    return [
      {
        line: DRIVER_LINES[history.provider] ?? history.provider,
        windowMinutes: history.windowMinutes,
        resetsAtMs: last[2],
        points: history.points
          .filter((point: readonly [number, number, number]) => point[2] === last[2])
          .map(([atMs, percentLeft]: readonly [number, number, number]) => ({ atMs, percentLeft })),
      },
    ];
  });
}

/**
 * The report from the two owner reads. An unavailable Digest read is the page's empty state with
 * its reason; an unavailable limits read only leaves the history out (the latest window still
 * shows), since `limits` needs Ops and a machine whose board never ran has no samples.
 */
export function reportFromAnswers(
  range: UsageRange,
  digest: OwnerReadAnswer<DigestPayload>,
  limits: OwnerReadAnswer<LimitsPayload>,
): DigestReport {
  if (digest.state !== 'ok') {
    const reason = digest.state === 'unavailable' ? digest.reason : digest.state;
    return { range, sample: false, unavailable: REASONS[reason] };
  }
  const document = digest.payload.report;
  if (digest.payload.since !== range || document?.since !== range)
    return { range, sample: false, unavailable: REASONS.unknown };
  const limitHistory = limitHistoryFrom(limits);
  return {
    range,
    sample: false,
    document: limitHistory ? { ...document, limitHistory } : document,
  };
}

/** The source the page reads: the owner's usage report and weekly-limit history. */
export const digestSource: DigestSource = {
  async report(range, signal = new AbortController().signal) {
    const [digest, limits] = await Promise.all([
      ownerRead<DigestPayload>(`digest/${range}`, signal),
      ownerRead<LimitsPayload>('limits', signal),
    ]);
    return reportFromAnswers(range, digest, limits);
  },
};
