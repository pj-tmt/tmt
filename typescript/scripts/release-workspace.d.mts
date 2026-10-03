import type { ComponentMap } from './ci-scope.mjs';
import type { Workspace } from './cargo-workspace.mjs';
export function validateReleaseWorkspace(input: {
  map: ComponentMap;
  workspace: Workspace;
}): string[];
