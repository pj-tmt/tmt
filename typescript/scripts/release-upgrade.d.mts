import type { DraftAsset, DraftRelease } from './release-draft-assets.mjs';

export function selectPrevious(input: {
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
  driver: string | null;
  files: Record<string, string>;
}
export function fetchUpgrade(input: {
  releases: readonly DraftRelease[];
  download: (asset: DraftAsset, file: string) => void;
  product: string;
  tag: string;
  directory: string;
}): UpgradePlan;
export function proveStaged(input: {
  directory: string;
  product: string;
  tag: string;
  target: string;
  run: (script: string, args: string[]) => void;
  skill?: string;
}): { previous: string | null };
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
