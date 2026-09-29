import type { ComponentMap } from './ci-scope.mjs';

export interface WorkspaceCrate {
  readonly name: string;
  /** Repository-relative path of the crate's Cargo.toml. */
  readonly manifest: string;
  /** Repository-relative directory of the crate. */
  readonly dir: string;
  /** Whether the crate takes its version from `[workspace.package]`. */
  readonly inheritsVersion: boolean;
  /** Names of the workspace crates it depends on outside dev-dependencies. */
  readonly dependencies: readonly string[];
}

export interface Workspace {
  readonly crates: readonly WorkspaceCrate[];
  /** Names that have an entry in `rust/Cargo.lock`. */
  readonly lockNames: ReadonlySet<string>;
  /** Every tracked file, repository-relative. */
  readonly files: readonly string[];
}

export interface ReleasePleaseExtraFile {
  readonly type: 'toml';
  readonly path: string;
  readonly jsonpath: string;
}

export interface ReleasePleasePackage {
  readonly 'release-type': string;
  readonly component: string;
  readonly 'include-component-in-tag': boolean;
  readonly prerelease: boolean;
  readonly 'exclude-paths'?: readonly string[];
  readonly 'extra-files': readonly ReleasePleaseExtraFile[];
}

export interface ReleasePleaseConfig {
  readonly $schema: string;
  readonly [option: string]: unknown;
  readonly packages: Readonly<Record<string, ReleasePleasePackage>>;
}

export function readWorkspace(root?: string): Workspace;
export function generateReleasePleaseConfig(input: {
  components: ComponentMap['components'];
  workspace: Workspace;
}): ReleasePleaseConfig;
export function renderReleasePleaseConfig(input: {
  components: ComponentMap['components'];
  workspace: Workspace;
}): string;
