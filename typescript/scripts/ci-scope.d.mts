export interface CiAreas {
  readonly native: boolean;
  readonly office: boolean;
  readonly nativeOffice: boolean;
}

export interface ComponentMap {
  readonly components: readonly {
    readonly name: string;
    readonly package?: string;
    readonly release?: boolean;
    readonly releaseConsumers: readonly string[];
    readonly owns: readonly string[];
    readonly excludes: readonly string[];
    readonly migrations: readonly string[];
    readonly selectedBy: readonly { readonly glob: string; readonly pattern: RegExp }[];
    readonly scopedChecks?: {
      readonly nativeTests: readonly string[];
      readonly e2eFiles: readonly string[];
    };
  }[];
  readonly rules: readonly {
    readonly id: string;
    readonly why: string;
    readonly consumers: readonly ('native' | 'office')[];
    readonly globs: readonly string[];
    readonly patterns: readonly RegExp[];
  }[];
  readonly digest: string;
}

export interface CiSelectionRow extends CiAreas {
  readonly path: string;
  readonly owner: string;
  readonly rule: string;
  readonly why: string;
}

export interface CiSelection {
  readonly paths: readonly string[];
  readonly rows: readonly CiSelectionRow[];
  readonly areas: CiAreas;
  readonly nativeScope: string;
  readonly digest: string;
}

export interface NativeJobResults {
  readonly nativeRust: string;
  readonly unitTests: string;
  readonly e2eShard1: string;
  readonly e2eShard2: string;
  readonly runtimeBuild: string;
  readonly packedInstall: string;
  readonly macosRuntimeBuild: string;
  readonly macosPackedInstall: string;
}

export interface E2eShardResults {
  readonly e2eShard1: string;
  readonly e2eShard2: string;
}

export function globToRegExp(glob: string): RegExp;
export function parseComponentMap(text: string): ComponentMap;
export function isReleased(map: ComponentMap, name: string): boolean;
export function ownerOf(path: string, map?: ComponentMap): string;
export function explainCiSelection(
  paths: readonly string[],
  map?: ComponentMap
): readonly CiSelectionRow[];
export function selectCiAreas(paths: readonly string[], map?: ComponentMap): CiAreas;
export function selectOfficeBrowser(paths: readonly string[], map?: ComponentMap): boolean;
export function selectNativeScope(paths: readonly string[], map?: ComponentMap): string;
export function scopedChecks(
  scope: string,
  map?: ComponentMap
): { readonly nativeTests: readonly string[]; readonly e2eFiles: readonly string[] };
export function nativeGatePasses(
  scope: string,
  results: NativeJobResults,
  macos: string,
  map?: ComponentMap
): boolean;
export function rustGatePasses(
  scope: string,
  results: readonly string[],
  map?: ComponentMap
): boolean;
export function e2eGatePasses(scope: string, results: E2eShardResults, map?: ComponentMap): boolean;
export function renderSelectionEvidence(input: {
  readonly base: string;
  readonly head: string;
  readonly rows: readonly CiSelectionRow[];
  readonly areas: CiAreas;
  readonly digest: string;
  readonly nativeScope?: string;
  readonly range?: '..' | '...';
}): string;
export function readChangedCiSelection(
  base: string,
  head: string,
  cwd: string,
  range?: '..' | '...'
): CiSelection;
export function readChangedCiAreas(base: string, head: string, cwd: string): CiAreas;
export function runCiScope(
  args: readonly string[],
  io: {
    readonly cwd: string;
    readonly stdout: { write(text: string): unknown };
    readonly stderr: { write(text: string): unknown };
    readonly summaryFile?: string;
  }
): void;
export function ciGatePasses(selected: string, results: readonly string[]): boolean;
export const NATIVE_OFFICE_UNREACHABLE: Readonly<
  Record<'tmt-adapters' | 'tmt-core', readonly string[]>
>;
