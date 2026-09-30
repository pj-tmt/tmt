export const BUNDLE_ASSET: string;
export const FAILURE_ASSET: string;
export const HOLD_ASSET: string;

export interface ReleaseAsset {
  readonly name: string;
}

/** The fields of GitHub's release object that the plan reads. */
export interface ReleaseObject {
  readonly draft: boolean;
  readonly tag_name: string;
  readonly target_commitish: string;
  readonly created_at: string;
  readonly assets?: readonly ReleaseAsset[];
}

export interface PlannedBuild {
  readonly tag: string;
  readonly sha: string;
  readonly createdAt: string;
}

export interface ReleasePlan {
  readonly builds: readonly PlannedBuild[];
  readonly awaiting: readonly PlannedBuild[];
  readonly blocked: readonly { readonly tag: string; readonly reason: string }[];
  readonly held: readonly { readonly tag: string }[];
}

export function planReleaseBuilds(input: {
  releases: readonly ReleaseObject[];
  product: string;
  retry?: string;
  hold?: string;
}): ReleasePlan;
export function renderPlanSummary(
  input: Omit<ReleasePlan, 'held' | 'awaiting'> & {
    held?: ReleasePlan['held'];
    awaiting?: ReleasePlan['awaiting'];
    product: string;
    retry?: string;
    hold?: string;
  }
): string;
export function releasesFrom(parsed: readonly unknown[]): ReleaseObject[];
