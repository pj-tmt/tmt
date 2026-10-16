export type GateEvent = "pull_request" | "merge_group";

export interface GateJob {
  readonly id: number;
  readonly name: string;
  readonly conclusion: string | null;
  readonly created_at: string;
  readonly started_at: string | null;
  readonly completed_at: string;
}

export interface GateRun {
  readonly id: number;
  readonly event: GateEvent;
  readonly createdAt: string;
  readonly attempt: number;
  readonly jobs: readonly GateJob[];
}

export interface Distribution {
  readonly n: number;
  readonly median: number | null;
  readonly p90: number | null;
  readonly max: number | null;
}

export interface JobSummary {
  readonly name: string;
  readonly wall: Distribution;
  readonly queue: Distribution;
}

export interface EventSummary {
  readonly runs: number;
  readonly measured: number;
  readonly retried: number;
  readonly firstCreatedAt: string | null;
  readonly lastCreatedAt: string | null;
  readonly gate: Distribution;
  readonly runnerMinutes: { readonly median: number | null; readonly p90: number | null };
  readonly jobs: readonly JobSummary[];
  readonly controlling: readonly { name: string; count: number; share: number }[];
}

export type GateSummary = Record<GateEvent, EventSummary>;

export interface Budgets {
  readonly gate: Record<GateEvent, { readonly median: number; readonly p90: number }>;
  readonly jobs: Record<string, Record<GateEvent, number>>;
}

export interface OverBudget {
  readonly event: GateEvent;
  readonly what: string;
  readonly value: number;
  readonly budget: number;
}

export type RestReader = (path: string) => unknown;

export const REQUIRED_CONTEXTS: readonly string[];
export const AGGREGATE_JOBS: readonly string[];
export function median(values: readonly number[]): number | null;
export function percentile(values: readonly number[], p: number): number | null;
export function ghReader(options: {
  repo?: string;
  maxRequests?: number;
  execute?: (command: string, args: string[], options: object) => string;
}): RestReader & { requests: () => number };
export function collectGateRuns(
  read: RestReader,
  options?: { workflow?: string; count?: number; events?: readonly GateEvent[] },
): GateRun[];
export function summarizeGate(runs: readonly GateRun[], required?: readonly string[]): GateSummary;
export function parseBudgets(markdown: string): Budgets;
export function overBudget(summary: GateSummary, budgets: Budgets): OverBudget[];
export function renderReport(
  summary: GateSummary,
  budgets: Budgets,
  options?: { workflow?: string },
): string;
