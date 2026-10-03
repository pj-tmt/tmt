import type { DraftEvidence } from './release-pr-safety.mjs';
import type { ComponentMap } from './ci-scope.mjs';
import type { runPackedCommand } from './packed-command.mjs';
export interface RestRecord {
  number?: number;
  pull_request?: object;
  id?: number;
  title?: string;
  state?: string;
  body?: string;
  tag_name?: string;
  draft?: boolean;
  created_at?: string;
  sha?: string;
  status?: string;
  head?: { ref: string; sha: string; repo?: { full_name: string } };
  base?: { ref: string };
  commit?: { committer?: { date: string } };
}
export interface StallClient {
  repository: string;
  get(path: string): RestRecord;
  list(path: string): RestRecord[];
  write(path: string, method: string, data: Record<string, string>): RestRecord;
  search(): RestRecord | undefined;
  git(args: string[]): string;
}
export interface ReleaseCommit {
  branch: string;
  sha: string;
  time: number;
}
export interface StallFinding {
  key: string;
  message: string;
}
export function createStallClient(
  options: {
    repository: string;
    token: string;
    cwd?: string;
    env?: NodeJS.ProcessEnv;
  },
  execute?: typeof runPackedCommand
): StallClient;
export function planReleaseCommits(input: {
  client: StallClient;
  components: ComponentMap['components'];
  releases: RestRecord[];
}): Promise<ReleaseCommit[]>;
export interface MonitorOptions {
  client: StallClient;
  manifest: Record<string, string>;
  components: ComponentMap['components'];
  heldPaths: string[];
  drafts: DraftEvidence[];
  queueSkipped: boolean;
  now?: number;
  live?: boolean;
  plan?: typeof planReleaseCommits;
}
export function detectReleaseStalls(
  options: MonitorOptions
): Promise<{ findings: StallFinding[]; warning?: string }>;
export function reconcileStallIssue(client: StallClient, findings: StallFinding[]): void;
export function monitorReleaseStalls(
  options: MonitorOptions,
  reporting?: { summarize?: (text: string) => void; warn?: (text: string) => void }
): Promise<{ findings: StallFinding[]; healthy: boolean; warning?: string }>;
