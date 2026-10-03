export const QUEUED_NOTICE: string;
export function queuedReleaseNotesCover(
  options: { repository?: string; token?: string; env?: NodeJS.ProcessEnv; cwd?: string },
  execute?: (
    executable: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }
  ) => string,
  checkNotes?: typeof import('./release-pr-safety.mjs').checkReleaseNotes
): Promise<boolean>;

export function prepareReleaseRefresh(
  options: Parameters<typeof queuedReleaseNotesCover>[0] & { live?: boolean; heldPaths?: string[] },
  execute?: Parameters<typeof queuedReleaseNotesCover>[1],
  checkNotes?: Parameters<typeof queuedReleaseNotesCover>[2]
): Promise<{ decision: 'run' | 'skip' | 'blocked'; notice?: string }>;

export function enableReleaseAutoMerge(
  options: Parameters<typeof queuedReleaseNotesCover>[0],
  execute?: Parameters<typeof queuedReleaseNotesCover>[1]
): string;
