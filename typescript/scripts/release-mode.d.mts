export function releaseMode(input: { event: string; ref?: string; dryRun?: string }): {
  live: boolean;
  reason: string;
};

export function pushCadence(
  input: { repository?: string; token?: string; runId?: string; now?: number },
  execute?: (
    executable: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }
  ) => string
): { live: boolean; reason: string };
