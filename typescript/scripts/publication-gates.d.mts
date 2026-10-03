import type { ComponentMap } from './ci-scope.mjs';
export const REQUIRED_CONTEXTS: readonly string[];
export const EARLY_GATES: readonly string[];
export const GATES: readonly string[];
export const UNSKIPPABLE_GATE: string;

export interface GateOutcome {
  readonly ok: boolean;
  readonly reason: string;
}
export interface GateResult extends GateOutcome {
  readonly gate: string;
  readonly skipped?: boolean;
}

export function checkCommit(input: {
  sha: string;
  onMain: boolean;
  pullRequest?: { number: number } | null;
  checkRuns: readonly {
    name: string;
    status: string;
    conclusion: string | null;
    completed_at: string;
  }[];
}): GateOutcome;
export function checkChannel(input: { product: string; tag: string }): GateOutcome;
export function checkImmutability(input: {
  releases: readonly {
    tag_name: string;
    draft?: boolean;
    published_at?: string;
    immutable?: boolean;
  }[];
}): GateOutcome;
export function checkMonotonic(input: {
  releases: readonly { tag_name: string; draft?: boolean }[];
  product: string;
  tag: string;
}): GateOutcome;
export function countMigrations(source: string): number;
export function isBreaking(input: { subject: string; body?: string; breaking?: boolean }): boolean;
export function releaseCommits(
  input: { from: string; to: string; product: string; map: ComponentMap },
  readGit?: (args: string[]) => string
): { sha: string; subject: string; body: string; breaking: boolean }[];
export function checkMigration(input: {
  files: readonly string[];
  counts: Record<string, number>;
  previous: { tag: string; counts: Record<string, number> } | null;
  commits: readonly { sha: string; subject: string; body?: string; breaking?: boolean }[];
  /** An alpha publishes new migrations; any other release is held for them. Defaults to false. */
  alpha?: boolean;
}): GateOutcome;
export function checkUpgrade(input: {
  result: string;
  outcome: string;
  reason?: string;
  url?: string;
}): GateOutcome;
export function runGates(input: {
  order: readonly string[];
  checks: Record<string, () => GateOutcome>;
  skip?: string;
}): { held: { gate: string; reason: string } | null; results: GateResult[] };
export function renderGateSummary(input: {
  tag: string;
  results: readonly GateResult[];
  held: { gate: string; reason: string } | null;
}): string;
