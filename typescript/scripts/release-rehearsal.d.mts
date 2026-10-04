import type { ComponentMap } from './ci-scope.mjs';
export function activeProducts(map?: ComponentMap): string[];
export function selectReleaseRehearsal(paths: readonly string[], map?: ComponentMap): string[];
export function runReleaseRehearsal(
  args: string[],
  io: {
    cwd: string;
    stdout: { write(text: string): unknown };
    stderr: { write(text: string): unknown };
    summaryFile?: string;
  }
): void;
