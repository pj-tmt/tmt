export function isRepository(value: unknown): value is string;
export function resolveRepository(options?: {
  repo?: string;
  env?: NodeJS.ProcessEnv;
  cwd?: string;
  execute?: (
    executable: string,
    args: string[],
    options: {
      env: NodeJS.ProcessEnv;
      cwd: string;
      timeoutMs: number;
      maxBuffer: number;
    }
  ) => string;
}): string;
