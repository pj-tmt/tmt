import type { ComponentMap } from './ci-scope.mjs';

export interface SafetyReader {
  readonly repository: string;
  get(path: string): unknown;
  list(path: string): unknown[];
  git(args: string[]): string;
}
export function createSafetyReader(
  options: { repository?: string; token?: string; cwd?: string; env?: NodeJS.ProcessEnv },
  execute?: (
    command: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }
  ) => string
): SafetyReader;
export function checkReleaseNotes(input: {
  pr: unknown;
  base?: string;
  components: ComponentMap['components'];
  reader: SafetyReader;
  releases?: unknown[];
}): { tag: string; linkedCommits: number } | null;
export function verifyReleasePrNotes(input: {
  eventName: string;
  event: unknown;
  components: ComponentMap['components'];
  reader: SafetyReader;
}): number;
/** Manifest paths held until matching draft tags exist. */
export function taglessDrafts(input: {
  manifest: unknown;
  components: ComponentMap['components'];
  reader: SafetyReader;
}): string[];

export interface QueueSubject {
  sha: string;
  title: string;
  number: number;
}
export function pendingQueueSubjects(input: {
  event: unknown;
  reader: Pick<SafetyReader, 'git'>;
}): QueueSubject[];
export function conventionalPrTitle(title: unknown): boolean;
export function checkQueueTitles(input: { event: unknown; reader: Pick<SafetyReader, 'git'> }): {
  checked: number;
  findings: QueueSubject[];
};
