import type { CargoWorkspace } from './cargo-workspace.mjs';
import type { ComponentMap } from './ci-scope.mjs';
export interface IssueEvidence {
  status: string;
  text: string;
  prs: number[];
  waiting: string[];
}
export interface ProjectItem {
  id: string;
  content: {
    __typename: 'Issue';
    id: string;
    number: number;
    url: string;
    state: string;
    repository: { nameWithOwner: string };
    labels: {
      nodes: { name: string }[];
      pageInfo: { hasNextPage: boolean; endCursor: string | null };
    };
  };
  status?: { name: string };
  released?: { text: string };
}
export interface Project {
  projectId: string;
  statusId: string;
  releasedId: string;
  options: Record<string, string>;
  pages: number;
  items: Map<string, ProjectItem>;
}
export interface Change extends IssueEvidence {
  itemId: string;
  issue: string;
  current: { status: string; text: string };
  writeText: boolean;
  writeStatus: boolean;
}
export interface Plan {
  rows: Change[];
  changes: Change[];
}
export interface Api {
  counts: { graphql: number; rest: number };
  points: { cost: number; remaining: number | null };
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
export interface ClosingPr {
  id: string;
  number: number;
  merged: boolean;
  mergeCommit: { oid: string };
  repository: { nameWithOwner: string };
}
export interface Closing {
  issues: Map<string, Set<string>>;
  prs: Map<string, ClosingPr>;
}
export interface GitEvidence {
  validateTags(tags: string[]): void;
  paths(sha: string): string[];
  containingTags(sha: string): Set<string>;
}
export interface Reconciliation {
  dryRun: boolean;
  releases: string[];
  issues: number;
  rows: Change[];
  changed: Change[];
  requests: Api['counts'];
  points: Api['points'];
}
export const PROJECT_ID: string;
export const LIMITS: { graphql: number; rest: number; pages: number; prs: number; batch: number };
export function releaseIdentity(
  tag: string
): { product: string; version: string; label: string } | undefined;
export function githubApi(options: {
  appToken?: string;
  readToken?: string;
  repository: string;
  spawn?: typeof import('node:child_process').spawnSync;
}): Api;
export function readReleases(api: Api): Release[];
export function readProject(api: Api, projectId?: string): Project;
export function readClosingPrs(api: Api, items: ProjectItem[], repository: string): Closing;
export function gitEvidence(options?: {
  cwd?: string;
  spawn?: typeof import('node:child_process').spawnSync;
}): GitEvidence;
export function affectedProducts(
  paths: string[],
  map: ComponentMap,
  workspace?: CargoWorkspace
): { products: string[]; unpublished: string[] };
export function deriveEvidence(
  items: ProjectItem[],
  closing: Closing,
  releases: Release[],
  git: GitEvidence,
  map: ComponentMap,
  workspace?: CargoWorkspace
): Map<string, IssueEvidence>;
export function planUpdates(evidence: Map<string, IssueEvidence>, project: Project): Plan;
export function applyUpdates(api: Api, project: Project, plan: Plan, dryRun: boolean): void;
export function reconcile(options: {
  api: Api;
  repository: string;
  dryRun: boolean;
  projectId?: string;
  reportReadbackMismatch?: (details: string) => void;
  git?: GitEvidence;
  map?: ComponentMap;
  workspace?: CargoWorkspace;
}): Reconciliation;
export function renderSummary(result: Reconciliation): string;
export function main(env?: NodeJS.ProcessEnv): void;

export function renderFailure(error: Error, api: Api): string;
