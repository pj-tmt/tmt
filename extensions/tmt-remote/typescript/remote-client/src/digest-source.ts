/**
 * What the Digest web view (`/digest/`) reads: per-agent token use over one range. The page depends
 * only on `DigestSource`; the source below is a MOCK until Core exposes a read API for the
 * owner's usage (`tmt digest usage report --json` holds the same facts on the terminal side).
 */
export const USAGE_RANGES = ['5m', '1h', '6h', '1d', '7d', '30d'] as const;
export type UsageRange = (typeof USAGE_RANGES)[number];

export interface AgentUsage {
  /** The agent's saved identity name. */
  agent: string;
  /** Input plus output tokens; cached input is part of input. */
  tokens: number;
  /** Assistant turns in the range, when the source counts them. */
  turns?: number;
}

export interface UsageReport {
  range: UsageRange;
  agents: readonly AgentUsage[];
  /** Whether these numbers are sample data, not the owner's usage. */
  sample: boolean;
  /** Why the source cannot report this range; the table is then empty. */
  unavailable?: string;
}

export interface DigestSource {
  usage(range: UsageRange): Promise<UsageReport>;
}

// MOCK: fixed sample numbers scaled by range, so the view can be built and reviewed before the
// Core read API exists. Replace with the API-backed source; nothing else in the page changes.
const SAMPLE: readonly Required<AgentUsage>[] = [
  { agent: 'tmt-ux-lead', tokens: 48_200_000, turns: 312 },
  { agent: 'ux-1', tokens: 31_000_000, turns: 205 },
  { agent: 'feat-1', tokens: 27_400_000, turns: 188 },
  { agent: 'remote-1', tokens: 19_900_000, turns: 141 },
  { agent: 'night-1', tokens: 6_300_000, turns: 52 },
];
const SCALE: Record<UsageRange, number> = {
  '5m': 1 / 2016,
  '1h': 1 / 168,
  '6h': 1 / 28,
  '1d': 1 / 7,
  '7d': 1,
  '30d': 30 / 7,
};
export const mockDigestSource: DigestSource = {
  async usage(range) {
    const scale = SCALE[range];
    return {
      range,
      sample: true,
      agents: SAMPLE.map((row) => ({
        agent: row.agent,
        tokens: Math.round(row.tokens * scale),
        turns: Math.round(row.turns * scale),
      })),
    };
  },
};

/** The source the page reads by default. */
export const digestSource: DigestSource = mockDigestSource;
