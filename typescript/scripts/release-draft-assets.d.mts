import type { ReleaseObject } from './plan-release-builds.mjs';

export interface DraftAsset {
  readonly id: number;
  readonly name: string;
  readonly digest?: string | null;
}

export interface DraftRelease extends ReleaseObject {
  readonly id: number;
  readonly assets?: readonly DraftAsset[];
}

export interface ReleaseApi {
  listReleases(): DraftRelease[];
  upload(release: DraftRelease, name: string, file: string): unknown;
  deleteAsset(id: number): unknown;
}

export function bundleFiles(
  product: string,
  manifest: { announcement_tag?: string; artifacts?: Record<string, { kind: string }> },
  tag: string
): string[];
export function checkDraft(input: { api: ReleaseApi; tag: string; retry?: boolean }): {
  todo: boolean;
  awaiting: boolean;
  reason: string;
};
export function attachBundle(input: {
  api: ReleaseApi;
  product: string;
  tag: string;
  directory: string;
  sleep?: (milliseconds: number) => void;
}): { uploaded: string[] };
export function recordFailure(input: {
  api: ReleaseApi;
  tag: string;
  runUrl: string;
  sha: string;
  jobs: readonly string[];
  now?: Date;
}): { recorded: boolean };
export function recordHold(input: {
  api: ReleaseApi;
  tag: string;
  hold: { gate: string; reason: string; sha?: string; runUrl?: string };
  now?: Date;
}): void;
export function clearHold(input: { api: ReleaseApi; tag: string }): boolean;
export function readHold(input: {
  api: ReleaseApi;
  tag: string;
  download: (asset: DraftAsset) => string;
}): { gate: string; reason: string } | null;
export function ghApi(input: {
  repository: string;
  env?: NodeJS.ProcessEnv;
  spawn?: (
    command: string,
    args: readonly string[],
    options: object
  ) => { error?: Error; status: number | null; stdout: string; stderr: string };
}): ReleaseApi;
