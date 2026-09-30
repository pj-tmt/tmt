export interface PublishedRelease {
  readonly tag_name: string;
  readonly draft?: boolean;
}

export function compareVersions(left: string, right: string): -1 | 0 | 1;
export function versionOfTag(tag: string, product: string): string;
export function publishedReleases<T extends PublishedRelease>(
  releases: readonly T[],
  product: string
): T[];
