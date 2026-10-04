import type { DraftAsset, DraftRelease } from './release-draft-assets.mjs';

export function selectPrevious(input: {
  releases: readonly DraftRelease[];
  product: string;
  candidateTag: string;
}): DraftRelease | null;
export function selectSupportFloor(input: {
  releases: readonly DraftRelease[];
  product: string;
  candidateTag: string;
}): DraftRelease | null;
export function archiveTargets(input: { release: DraftRelease; product: string }): string[];
export function selectAssets(input: { release: DraftRelease; product: string; target: string }): {
  archive: DraftAsset;
  manifest: DraftAsset;
};
export function stageRelease(input: {
  download: (asset: DraftAsset, file: string) => void;
  release: DraftRelease;
  product: string;
  target: string;
  directory: string;
}): { archive: string; manifest: string; digests: Record<string, string> };

export interface UpgradePlan {
  product: string;
  tag: string;
  previous: string | null;
  floor: string | null;
  driver: string | null;
  files: Record<string, string>;
}
export interface LocalCandidate {
  release: DraftRelease;
  download: (asset: DraftAsset, file: string) => void;
}
export function localCandidate(input: {
  directory: string;
  product: string;
  tag: string;
}): LocalCandidate;
export function fetchUpgrade(input: {
  releases: readonly DraftRelease[];
  download: (asset: DraftAsset, file: string) => void;
  product: string;
  tag: string;
  directory: string;
  /** A rehearsal's verified local bundle replaces the candidate release. */
  local?: LocalCandidate;
}): UpgradePlan;
export function proveStaged(input: {
  directory: string;
  product: string;
  tag: string;
  target: string;
  run: (script: string, args: string[]) => void;
  skill?: string;
  sourceRoot?: string;
}): { previous: string | null };
export const ACCEPTANCE_TEST: string;
export function acceptanceApplicability(sourceRoot: string): 'applicable' | 'predates';
export function proveArchiveAcceptance(input: {
  directory: string;
  product: string;
  tag: string;
  target: string;
  sourceRoot?: string;
  execute?: (
    executable: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }
  ) => string;
  environment?: NodeJS.ProcessEnv;
  report?: (message: string) => void;
}): { outcome: 'nothing' | 'predates' | 'proved' };
export const PROOF_FILES: readonly string[];
export function assessUpgrade(input: {
  plan: { previous: string | null; product?: string };
  hasFileAt: (file: string) => boolean;
}): { outcome: 'nothing' | 'predates' | 'proved'; reason: string };
export function releaseCommit(input: {
  release: DraftRelease;
  commitOfTag: (tag: string) => string;
}): string;
export function ghAssetDownloader(input: {
  repository: string;
  env?: NodeJS.ProcessEnv;
  spawn?: (
    command: string,
    args: string[],
    options: object
  ) => { error?: Error; status: number | null; stdout: Buffer; stderr: Buffer };
}): (asset: DraftAsset, file: string) => void;
export function failureCause(log: string): string;
export function combineFailures(failures: readonly { target: string; log: string }[]): string;
