import type { runPackedCommand } from './packed-command.mjs';
export type DependencyKind = 'normal' | 'build' | 'dev';
export interface CargoPackage {
  /** Cargo-owned target facts; optional only for existing injected projections. */
  readonly id?: string;
  readonly targets?: readonly CargoTarget[];
  readonly name: string;
  readonly version: string;
  readonly manifestPath: string;
  readonly manifest: string;
  readonly dir: string;
  readonly binTargets: readonly string[];
  readonly distMetadata: Readonly<Record<string, unknown>>;
  readonly dependencies: Readonly<Record<DependencyKind, readonly string[]>>;
}
export interface CargoTarget {
  readonly name: string;
  readonly kind: readonly string[];
  readonly crate_types: readonly string[];
  readonly src_path: string;
  readonly test: boolean;
  readonly doctest: boolean;
  readonly edition: string;
  readonly 'required-features'?: readonly string[];
}
export interface CargoWorkspace {
  readonly packages: readonly CargoPackage[];
  /** Includes the selected package itself, visiting each workspace package once. */
  closure(name: string, kinds: readonly DependencyKind[]): readonly CargoPackage[];
}
export function readCargoWorkspace(
  root: string,
  options?: { runner?: typeof runPackedCommand }
): CargoWorkspace;

/** Caller-owned capture may use these same arguments without changing the reader API. */
export function cargoWorkspaceCommand(root: string): {
  readonly executable: string;
  readonly args: readonly string[];
  readonly options: {
    readonly cwd: string;
    readonly env: NodeJS.ProcessEnv;
    readonly timeoutMs: number;
  };
};
