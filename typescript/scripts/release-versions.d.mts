/** Synthetic version for non-publishing rehearsals and installation fixtures. */
export function syntheticAlphaVersion(committedVersion: string): string;
export interface PublishedRelease {
  readonly tag_name: string;
  readonly draft?: boolean;
}

export function compareVersions(left: string, right: string): -1 | 0 | 1;
export function isAlphaVersion(version: string): boolean;
export function versionOfTag(tag: string, product: string): string;
export function publishedReleases<T extends PublishedRelease>(
  releases: readonly T[],
  product: string
): T[];
