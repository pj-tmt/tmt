export const QUEUED_NOTICE: string;
export function releasePrQueued(
  options: { repository?: string; token?: string; env?: NodeJS.ProcessEnv; cwd?: string },
  execute?: (
    executable: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }
  ) => string,
  checkNotes?: typeof import('./release-pr-safety.mjs').checkReleaseNotes
): Promise<boolean>;

export function enableReleaseAutoMerge(
  options: Parameters<typeof releasePrQueued>[0],
  execute?: Parameters<typeof releasePrQueued>[1]
): string;
