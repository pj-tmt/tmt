import type { runPackedCommand } from './packed-command.mjs';
export type DependencyKind = 'normal' | 'build' | 'dev';
export interface CargoPackage {
  readonly name: string;
  readonly version: string;
  readonly manifestPath: string;
  readonly manifest: string;
  readonly dir: string;
  readonly binTargets: readonly string[];
  readonly distMetadata: Readonly<Record<string, unknown>>;
  readonly dependencies: Readonly<Record<DependencyKind, readonly string[]>>;
}
export interface CargoWorkspace {
  readonly packages: readonly CargoPackage[];
  /** Includes the selected package itself, visiting each workspace package once. */
  closure(name: string, kinds: readonly DependencyKind[]): readonly CargoPackage[];
}
export function readCargoWorkspace(root: string, options?: { runner?: typeof runPackedCommand }): CargoWorkspace;
