export const DEPENDENCY_PREFIX: string;
export function dependencyCacheKey(root: string, files?: readonly string[]): string;
export function inspectDependencyCache(
  bundle: string,
  key: string
): { usable: boolean; reason: string };
export function renderDependencyDockerfile(text: string): string;
