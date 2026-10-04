import type { ComponentMap } from './ci-scope.mjs';

export interface QueueReader {
  git(args: string[]): string;
}
export interface QueueTitle {
  sha: string;
  title: string;
  number: number;
}
export function pendingQueueSubjects(input: { event: unknown; reader: QueueReader }): QueueTitle[];
export function conventionalPrTitle(title: unknown): boolean;
export function checkQueueTitles(input: { event: unknown; reader: QueueReader }): {
  checked: number;
  findings: QueueTitle[];
};
export function checkPullRequestTitle(input: {
  title: unknown;
  paths: string[];
  map: ComponentMap;
}): { components: string[]; ok: boolean };
