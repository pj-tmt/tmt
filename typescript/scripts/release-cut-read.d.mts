import type { ReleaseObject } from './plan-release-builds.mjs';

export function readCutMetadata(
  input: {
    repository: string;
    cut: string;
    token: string;
    draftVisibility: string;
  },
  execute?: (
    executable: string,
    args: string[],
    options: {
      cwd: string;
      env: NodeJS.ProcessEnv;
      timeoutMs: number;
    }
  ) => string
): {
  schema: number;
  repository: string;
  cut: string;
  draftVisibility: string;
  capturedAt: string;
  releases: {
    id: number;
    tag_name: string;
    draft: boolean;
    body?: string;
    target_commitish?: string;
    assets: { name: string }[];
  }[];
};

export function readReleaseCatalog(input: { repository: string; token?: string }, execute?: Parameters<typeof readCutMetadata>[1]): ReturnType<typeof readCutMetadata>['releases'];

export function createReleaseReader(input: { repository: string; token?: string; sleep?: (milliseconds: number) => void }, execute?: Parameters<typeof readCutMetadata>[1]): {
  byTag(tag: string): ReleaseDetails | null;
  byId(id: number): ReleaseDetails;
};

export interface ReleaseDetails extends ReleaseObject {
  readonly id: number;
  readonly assets: readonly { readonly id: number; readonly name: string; readonly digest?: string | null }[];
}
