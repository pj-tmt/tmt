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
export function readRetryHosts(
  directory: string,
  prefix?: string,
  runAttempt?: number
): RetryHost[];
export function planSmokeRetry(hosts: readonly RetryHost[], options?: { now?: number }): RetryPlan;
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
