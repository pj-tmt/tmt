export interface IssueEvidence {
  id: string;
  number: number;
  url: string;
  labels: Set<string>;
}
export interface ProjectItem {
  id: string;
  content: { id: string };
  status?: { name: string };
  released?: { text: string };
}
export interface Project {
  projectId: string;
  statusId: string;
  releasedId: string;
  optionId: string;
  pages: number;
  items: Map<string, ProjectItem>;
}
export interface Change {
  itemId: string;
  issue: string;
  text: string;
  writeText: boolean;
  writeStatus: boolean;
}
export interface Plan {
  changes: Change[];
  skipped: string[];
}
export interface Api {
  counts: { graphql: number; rest: number };
  rest(path: string): unknown;
  graphql(query: string): unknown;
  reserve(requests: number): void;
}
export interface Release {
  tag_name: string;
  published_at: string | null;
  draft: boolean;
  body?: string;
}
export const PROJECT_ID: string;
export const LIMITS: {
  graphql: number;
  rest: number;
  pages: number;
  releases: number;
  prs: number;
  batch: number;
};
export function releaseIdentity(
  tag: string
): { product: string; version: string; label: string } | undefined;
export function noteReferences(body: string, repository: string): number[];
export function githubApi(options: {
  projectToken?: string;
  readToken?: string;
  repository: string;
  spawn?: typeof import('node:child_process').spawnSync;
}): Api;
export function publishedWindow(
  releases: Release[],
  eventName: string,
  event: unknown,
  tag?: string
): Release[];
export function readReleases(api: Api): Release[];
export function resolveClosingIssues(
  api: Api,
  repository: string,
  numbers: number[]
): Map<number, Map<string, Omit<IssueEvidence, 'labels'>> | null>;
export function releaseIssues(
  api: Api,
  repository: string,
  selected: Release[],
  releases: Release[]
): {
  issues: Map<string, IssueEvidence>;
  sources: { tag: string; method: string; prs: number[] }[];
};
export function readProject(api: Api, projectId?: string): Project;
export function planUpdates(issues: Map<string, IssueEvidence>, project: Project): Plan;
export function applyUpdates(api: Api, project: Project, plan: Plan, dryRun: boolean): void;
export function reconcile(options: {
  api: Api;
  repository: string;
  eventName: string;
  event: unknown;
  tag?: string;
  dryRun: boolean;
  projectId?: string;
}): {
  dryRun: boolean;
  releases: unknown[];
  issues: number;
  changed: Change[];
  outsideProject: string[];
  requests: Api['counts'];
};
