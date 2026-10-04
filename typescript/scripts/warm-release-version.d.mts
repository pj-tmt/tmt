export function captureStallDiagnostics(
  pid: number | undefined,
  options?: {
    platform?: string;
    run?: (
      command: string,
      args: string[],
      options: object
    ) => { stdout?: string; stderr?: string; error?: Error };
  }
): string;
export function warmReleaseVersion(options?: {
  tool?: string;
  waitMs?: number;
  tickMs?: number;
  diagnoseAtMs?: number;
  secondBoundMs?: number;
  log?: (line: string) => void;
  diagnose?: (pid: number | undefined) => string;
  spawnProcess?: typeof import('node:child_process').spawn;
}): Promise<{ firstMs: number; secondMs: number }>;
