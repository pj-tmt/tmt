export interface WatchRun {
  id: number;
  run_attempt: number;
  workflow_id: number;
  path: string;
  name: string;
  event: string;
  head_branch: string;
  status: string;
  conclusion: string | null;
  created_at: string;
  updated_at: string;
  html_url: string;
}
export interface RoutingMap {
  source: string;
  rules: { workflowPrefix?: string; jobPrefix?: string; squad: string; source: string }[];
}
export interface WatchAlert {
  signature: string;
  workflow: string;
  workflowPath: string;
  event: string;
  job: string;
  route: { squad: string; source: string };
  firstSeen: number;
  lastSeen: number;
  lastFailure: number;
  count: number;
  runUrl: string;
  runId?: number;
  attempt?: number;
  pending: boolean;
  issueUrl?: string;
  lastNotifiedAt?: number;
  lastRequestId?: string;
  detectionDelaySeconds?: number;
  coverage?: boolean;
}
export interface WatchState {
  version: number;
  cursor: string;
  pages: Record<string, { etag?: string; total: number; rows: WatchRun[] }>;
  tracked: Record<string, WatchRun>;
  seen: Record<string, number>;
  alerts: Record<string, WatchAlert>;
  green: Record<string, number>;
  notifications: number[];
  coverage: Record<string, number>;
  delivery: { signatures: string[]; startedAt: number; message: string } | null;
  issueIntent: string | null;
}
export const WATCH_LIMITS: Readonly<{
  intervalMs: number;
  lookbackMs: number;
  dedupMs: number;
  notificationsPerHour: number;
  maxReads: number;
  coreFloor: number;
  deadlineMs: number;
  maxPages: number;
  maxSignatures: number;
}>;
export interface WatchApi {
  get(endpoint: string, etag?: string): { status: number; etag?: string; body: unknown };
}
export function newWatchState(now: number): WatchState;
export function validateRouting(map: unknown): RoutingMap;
export function routeFailure(
  map: RoutingMap,
  workflow: string,
  job: string
): { squad: string; source: string };
export function watchedRun(run: WatchRun): boolean;
export function failureSignature(run: WatchRun, job: string): string;
export function observeRun(
  state: WatchState,
  run: WatchRun,
  jobs: { id?: number; name: string; conclusion: string }[],
  routing: RoutingMap,
  now: number
): void;
export function heartbeatStatus(
  heartbeat: unknown,
  now: number
): { healthy: boolean; ageMs: number | null };
export function notificationGroups(state: WatchState, now: number): WatchAlert[][];
export function formatNotification(group: WatchAlert[]): string;
export function parseGhResponse(text: string): {
  status: number;
  etag?: string;
  remaining?: number;
  resource?: string;
  body: unknown;
};
export function collectWatchCycle(options: {
  state: WatchState;
  api: WatchApi;
  routing: RoutingMap;
  repo?: string;
  now: number;
  initial?: boolean;
  dryRun?: boolean;
}): {
  state: WatchState;
  examined: { run: number; attempt: number; conclusion: string }[];
  groups: WatchAlert[][];
};
export function deliverWatchAlerts(
  state: WatchState,
  effects: {
    findIssues(signature: string): string[];
    createIssue(alert: WatchAlert): string;
    talk(message: string): string;
    save(state: WatchState): void;
    reserve?(milliseconds: number): void;
  },
  now: number
): void;
export function createWatchApi(options?: {
  execute?: (executable: string, args: string[], timeoutMs?: number) => string;
  clock?: () => number;
  progress?: () => void;
}): WatchApi & { readonly calls: number; readonly ledger: unknown[]; readonly remainingMs: number };

export const WATCH_STATE_ENV: 'TMT_CI_WATCH_STATE';
export interface WatchHeartbeatEvidence {
  checkedAt?: string;
  path: string | null;
  heartbeat?: unknown;
  error?: string;
}
export function watchHeartbeatPath(directory?: string): string | null;
export function readWatchHeartbeat(
  directory?: string,
  read?: (file: string, encoding: 'utf8') => string
): WatchHeartbeatEvidence;
export function watchHeartbeat(
  state: WatchState,
  now: number,
  status: string,
  extra?: Record<string, unknown>
): {
  updatedAt: string;
  status: string;
  cursor: string;
  pending: number;
  coverageGaps: { updatedAt: string; message: string }[];
};
