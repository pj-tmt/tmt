export interface WorkflowSource {
  path: string;
  text: string;
}
export interface SourceTrace {
  path: string;
  macos: boolean;
  evidence: string[];
}
export interface CoreRate {
  remaining: number;
  limit: number;
  reset: number;
}
export interface RequestLedger {
  calls: number;
  cap: number;
  coreBefore: CoreRate;
  coreAfter: CoreRate;
  ledger: { path: string; attempt: number; outcome: string }[];
}
export interface DemandReader {
  (path: string): unknown;
  requests(): number;
  remaining(): number;
  finish(): RequestLedger;
}
export interface DemandRow {
  workflow: string;
  path: string;
  event: string;
  labels: string[];
  jobs: number;
  completed: number;
  skipped: number;
  pending: number;
  unfinished: number;
  invalidTimings: number;
  durationSamples: number;
  runnerMinutes: number | null;
  queueSeconds: { n: number; median: number | null; p90: number | null; max: number | null };
}
export interface DemandReport {
  schema: number;
  complete: boolean;
  since: string;
  until: string;
  sourceSha?: string;
  cohort: string;
  workflows: (SourceTrace & { id: number; name: string; reason: string })[];
  runs: number;
  actualExecutions: number;
  retainedCopies: number;
  selfHostedJobs: number;
  unclassifiedJobs: number;
  rows: DemandRow[];
  jobRows: (DemandRow & { name: string })[];
  prRows: (Omit<DemandRow, 'workflow' | 'path' | 'event' | 'labels'> & { association: string })[];
  measurements: {
    run: number;
    job: number;
    name: string;
    workflow: string;
    path: string;
    prs: number[];
    association: string;
    skipped: number;
    pending: number;
    completed: number;
    unfinished: number;
    invalidTimings: number;
    event: string;
    labels: string[];
    status: string;
    conclusion: string | null;
    startedAt: string | null;
    queueSeconds: number | null;
    runnerMinutes: number | null;
  }[];
  requests?: RequestLedger;
}
export function traceWorkflows(sources: WorkflowSource[]): SourceTrace[];
export function demandReader(options?: {
  repo?: string;
  maxRequests?: number;
  execute?: (command: string, args: string[], options: object) => string;
}): DemandReader;
export function collectDemand(
  read: Pick<DemandReader, 'remaining'> & ((path: string) => unknown),
  options: {
    since: string;
    until: string;
    sources: WorkflowSource[];
    historicalSources?: (path: string, sha: string) => WorkflowSource[];
  }
): DemandReport;
export function renderDemand(report: DemandReport): string;
