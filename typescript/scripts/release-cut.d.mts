import type { CargoWorkspace } from './cargo-workspace.mjs';
import type { ComponentMap } from './ci-scope.mjs';
export interface CutCommit {
  sha: string;
  message: string;
  files: string[];
}
export interface CutMetadata {
  schema: number;
  repository: string;
  cut: string;
  draftVisibility?: string;
  capturedAt?: string;
  evidenceError?: string;
  releases?: { tag_name: string; draft: boolean; body?: string; target_commitish?: string }[];
  runs?: { id: number; status: string; display_title: string }[];
}
export interface CutRow {
  product: string;
  cut: string;
  status: string;
  reason?: string;
  tag?: string;
  version?: string;
  previous?: string;
  previousTag?: string;
  notes?: string;
  commits?: string[];
  breaking?: boolean;
  authorization?: string;
}
export interface CutPlan {
  cut: string;
  repository: string;
  mode: 'plan';
  mapDigest?: string;
  unavailable?: string;
  components: CutRow[];
}
export function parseReleaseCommits(commits: CutCommit[]): {
  hash: string;
  header: string;
  subject: string;
  type: string;
  scope: string | null;
  notes: { title: string; text: string }[];
}[];
export function attributeCutCommits(
  commits: CutCommit[],
  map: ComponentMap,
  product: string,
  workspace?: CargoWorkspace
): CutCommit[];
export function nextAlphaVersion(version: string): string;
export function renderCutNotes(input: {
  commits: CutCommit[];
  repository: string;
  version: string;
  previousTag?: string;
  tag: string;
  date?: string;
}): Promise<{ notes: string; commits: string[]; breaking: boolean }>;
export function readCutRange(
  git: (args: string[]) => string,
  previous: string,
  cut: string
): CutCommit[];
export function releaseCutHistory(input: {
  releases: NonNullable<CutMetadata['releases']>;
  product: string;
  cut: string;
  git: (args: string[]) => string;
  excludeTag?: string;
}): { highestVersion: string | undefined; previous: { tag: string; sha: string } | null };
export function planReleaseCuts(input: {
  metadata: CutMetadata;
  map: ComponentMap;
  workspace?: CargoWorkspace;
  git: (args: string[]) => string;
  date?: string;
  workspace?: CargoWorkspace;
  initialVersions?: Record<string, string>;
  /** Owner-dispatched product versions; native publication authorization remains independent. */
  versions?: Record<string, string>;
}): Promise<CutPlan>;
export function renderCutSummary(plan: CutPlan): string;
