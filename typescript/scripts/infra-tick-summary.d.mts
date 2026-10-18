export const QUEUE_QUERY: string;
export const COVERAGE: string;
export interface TickApi {
  get(endpoint: string, immutable?: boolean, text?: boolean): unknown;
  releases(): unknown[];
  queue(): unknown;
  qualifyTerminalJobs?(paths: string[]): void;
  evidence?(): { requests: number; queueQueries?: number; files?: string[] };
}
export interface TickOptions {
  repository: string;
  since: string;
  until: string;
  previous?: {
    schema: number;
    until: string;
    failedDraftIds: number[];
    failedRunAttempts: string[];
    activeRunIds: number[];
    failureEvidence?: TickReport['failures'];
    baselineUntil?: string;
  } | null;
  reviewMap: { rules: { prefix: string; source: string; leads: string[] }[] };
  local: {
    guides: Record<string, string>;
    budgets: { lines: Record<string, number>; bytes: Record<string, number> };
    freeBytes: number;
    load1: number;
  };
}
export interface TickReport {
  schema: number;
  repository: string;
  since: string;
  until: string;
  coverage: string;
  complete: boolean;
  anomalies: ({ kind: string } & Record<string, unknown>)[];
  health: { created24h: number; createdSinceTick: number };
  baseline: string;
  olderActive: unknown[];
  trackedCompletions: unknown[];
  failures: {
    run: number;
    attempt: number;
    created: string;
    context: string | null;
    worker: string;
    tests: string[];
    logHash: string | null;
  }[];
  repeated: { test: string; occurrences: unknown[] }[];
  queue: { pr: number; position: number; state: string; ageMinutes: number }[];
  failedDrafts: { id: number; tag: string }[];
  postPublicationFailures: {
    tag: string;
    releaseId: number;
    run: number;
    attempt: number;
    worker: string;
    url: string;
  }[];
  failedRunAttempts: string[];
  activeRunIds: number[];
  reviewEvidence: {
    pr: number;
    head: string;
    shared: string[];
    statements: { lead: string; head: string; comment: string; url: string }[];
    disposition: string;
  }[];
  guides: unknown[];
  host: { freeBytes: number; load1: number };
  evidence: { requests?: number; queueQueries?: number; files?: string[] };
}
export function collectInfraTick(api: TickApi, options: TickOptions): TickReport;
export function renderInfraTick(report: TickReport): string;
export function failingTests(log: string): string[];
export function createTickClient(options: {
  repository: string;
  cache: string;
  clock?: () => number;
  trace?: (entry: {
    request: number;
    method: string;
    endpoint: string | undefined;
    outcome: string;
    milliseconds: number;
  }) => void;
  execute?: (
    program: string,
    args: string[],
    options: { timeoutMs: number; [key: string]: unknown }
  ) => string;
}): TickApi & { evidence(): { requests: number; queueQueries: number; files: string[] } };
