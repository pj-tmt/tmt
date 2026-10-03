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
