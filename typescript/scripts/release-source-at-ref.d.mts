import type { ComponentMap } from './ci-scope.mjs';
import type { CargoWorkspace } from './cargo-workspace.mjs';
export interface ReleaseSourceOptions {
  root?: string;
  execute?: (
    command: string,
    args: string[],
    options: {
      cwd: string;
      env: NodeJS.ProcessEnv;
      timeoutMs: number;
    }
  ) => string;
}
/** The callback reads only the exported ref; its directory is removed before this returns. */
export function withReleaseSourceAtRef<T>(
  sha: string,
  read: (sourceRoot: string) => T,
  options?: ReleaseSourceOptions
): T;

export function readReleaseSourceAtRef(
  sha: string,
  options?: ReleaseSourceOptions & { warm?: boolean }
): {
  map: ComponentMap;
  workspace: CargoWorkspace;
};
