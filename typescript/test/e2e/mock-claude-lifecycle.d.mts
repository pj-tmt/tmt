export function hookResult(
  error: Error | null,
  stdout: string,
  stderr: string
): { stdout: string; diagnostic?: string };

export function waitForLifecycle(
  databasePath: string,
  parentPid: number,
  sessionId: string,
  phase: 'running' | 'started' | 'working',
  deadline: number
): Promise<void>;
