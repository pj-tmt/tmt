import type { SmokeResult } from './verify-public-install.mjs';
import type { SmokeRecoveryApi } from './release-publish.mjs';

export interface RetryHost {
  product: string;
  tag: string;
  target: string;
  runAttempt: number;
  failed: readonly Omit<SmokeResult, 'ok'>[];
}
export interface RetryPlan {
  groups: { product: string; tag: string; hosts: RetryHost[]; targets: string[] }[];
  matrix: { include: { product: string; tag: string; target: string; runner: string }[] };
  waitUntilMs: number;
  skipped: { product: string; tag: string; target: string; reason: string }[];
}
export interface RetryRequest {
  sourceRunId: number | string;
  sourceRunAttempt: number | string;
  product: string;
  tag: string;
  targets: readonly string[] | string;
}
export interface RetrySource {
  run: {
    id: number;
    run_attempt: number;
    head_branch: string;
    event: string;
    path: string;
    repository: { full_name: string };
    head_repository: { full_name: string };
  };
  jobs: { name: string; conclusion: string | null }[];
}
export function parseRetryRequest(input: RetryRequest): RetryRequest & { targets: string[] };
export function validateRetrySource(
  input: RetrySource & {
    request: RetryRequest;
    repository: string;
    hosts: readonly RetryHost[];
    now?: number;
  }
): RetryPlan;
export function ghSmokeRetryApi(input: {
  repository: string;
  env?: NodeJS.ProcessEnv;
  spawn?: (
    command: string,
    args: readonly string[],
    options: object
  ) => { error?: Error; status: number | null; stdout: string; stderr: string };
}): {
  readSource(request: RetryRequest): RetrySource;
  dispatch(request: RetryRequest): void;
};
export function readRetryHosts(
  directory: string,
  prefix?: string,
  runAttempt?: number
): RetryHost[];
export function planSmokeRetry(
  hosts: readonly RetryHost[],
  options?: { now?: number; targets?: readonly string[] }
): RetryPlan;
export function waitForSmokeReset(
  plan: RetryPlan,
  options?: {
    now?: () => number;
    wait?: (milliseconds: number) => Promise<void>;
  }
): Promise<void>;
export function reportSmokeRetry(input: {
  api: SmokeRecoveryApi;
  plan: RetryPlan;
  retried: readonly RetryHost[];
  originalRunUrl: string;
  retryRunUrl: string;
}): { tag: string; ok: boolean }[];
