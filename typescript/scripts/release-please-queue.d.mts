export const QUEUED_NOTICE: string;
export function releasePrQueued(
  options: { repository?: string; token?: string; env?: NodeJS.ProcessEnv },
  execute?: (
    executable: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }
  ) => string
): boolean;
