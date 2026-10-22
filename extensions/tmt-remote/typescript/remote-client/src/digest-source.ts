/**
 * What the Digest web view (`/digest/`) reads: the usage report the terminal Digest shows. The
 * types follow the public document of `tmt digest usage report --json` (digest `usage-v1.md`) and
 * the exchange flow of `tmt x flow --json`; the page depends only on `DigestSource`. Until Core
 * serves that read to the owner's browser (#4325) the product source answers "not in the
 * browser yet"; the seeded sample lives in `digest-mock.ts` for tests, the gallery and dev only.
 */
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
 * Percent left of a weekly window sampled over time. NOT in the public report today: the page
 * asks #4325 for it, and without it shows the latest window only.
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
  report(range: UsageRange): Promise<DigestReport>;
}

/** The source the page reads by default. */
// Needs a Core read of Digest usage served to the owner's browser (#4325).
export const digestSource: DigestSource = {
  async report(range) {
    // The terminal board has a Digest tab; a person reads it better than the JSON report.
    const unavailable = 'Usage is not in the browser yet. Open Digest in `tmt ops ui`.';
    return { range, sample: false, unavailable };
  },
};
