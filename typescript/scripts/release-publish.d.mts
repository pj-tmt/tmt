import type { ReleaseApi } from './release-draft-assets.mjs';

export interface PublishApi extends ReleaseApi {
  latestRelease(): { tag_name: string } | null;
  setLatest(tag: string): void;
  publish(tag: string, flags: readonly string[]): void;
}

/** What `gh` reports for a check that can fail without throwing. */
export interface Outcome {
  readonly ok: boolean;
  readonly output: string;
}

/** The fields of a published release that the checks read. */
export interface PublishedRelease {
  readonly draft: boolean;
  readonly immutable?: boolean;
  readonly prerelease: boolean;
  readonly tag_name: string;
  readonly target_commitish: string;
  readonly assets?: readonly { readonly name: string }[];
}

export interface PublishedApi {
  listReleases(): readonly { tag_name: string; draft: boolean }[];
  getRelease(tag: string): PublishedRelease | null;
  latestRelease(): PublishedRelease | null;
  tagCommit(tag: string): string | null;
  download(tag: string, directory: string): Outcome;
  verifyRelease(tag: string): Outcome;
  verifyAsset(tag: string, file: string): Outcome;
}

export interface IssueApi {
  openIssue(title: string): number | null;
  createIssue(title: string, body: string): number;
  commentIssue(number: number, body: string): void;
}
export interface CheckResult {
  readonly check: string;
  readonly ok: boolean;
  readonly reason: string;
}

export function publishBlocker(input: {
  release: (Partial<PublishedRelease> & { draft?: boolean }) | undefined;
  product: string;
  tag: string;
  released?: boolean;
}): string;
export function convergeCliLatest(input: {
  api: PublishApi;
  tag: string;
  attempts?: number;
  wait?: () => void;
}): string;
export function publishDraft(input: {
  api: PublishApi;
  product: string;
  tag: string;
  released?: boolean;
  wait?: () => void;
}): {
  flags: string[];
};
export function checkPublishedRelease(input: {
  release: PublishedRelease;
  latest: PublishedRelease | null;
  highestCliTag?: string | null;
  tagCommit: string | null;
  product: string;
  tag: string;
}): CheckResult[];
export function verifyPublication(input: {
  api: PublishedApi;
  product: string;
  tag: string;
  directory: string;
  attempts?: number;
  sleep?: (milliseconds: number) => void;
}): CheckResult[];
export function postPublicationIssueTitles(tag: string): { failure: string; rateLimit: string };
export function renderFailureIssue(input: {
  tag: string;
  results: readonly CheckResult[];
  runUrl?: string;
}): { title: string; body: string };
export function reportFailure(input: {
  api: IssueApi;
  tag: string;
  results: readonly CheckResult[];
  runUrl?: string;
}): { issue: number; created: boolean };
export function readSmokeFailures(
  directory: string,
  options?: { expectedResults?: number; artifactPrefix?: string }
): CheckResult[];
export function renderVerifySummary(input: {
  tag: string;
  results: readonly CheckResult[];
}): string;
export function ghPublishApi(input: {
  repository: string;
  env?: NodeJS.ProcessEnv;
  spawn?: (
    command: string,
    args: readonly string[],
    options: object
  ) => { error?: Error; status: number | null; stdout: string; stderr: string };
}): PublishApi & PublishedApi & IssueApi;
