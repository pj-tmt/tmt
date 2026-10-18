export interface ShardWeights {
  readonly defaultSeconds: number;
  readonly files: Readonly<Record<string, number>>;
}

export interface FileShard {
  readonly files: string[];
  readonly seconds: number;
}

export interface SpecSuite {
  readonly directory: URL;
  readonly weightsFile: URL;
  readonly specPattern: string;
  readonly prefix: string;
}

export function listSpecFiles(directory: URL, specPattern: string): string[];
export function splitSpecFiles(
  files: readonly string[],
  count: number,
  weights: ShardWeights
): FileShard[];
export function specShardFiles(
  index: number,
  count: number,
  suite: SpecSuite,
  files?: readonly string[]
): string[];

export const ACCEPTANCE_SHARDS: number;
export const COMPONENT_SHARDS: number;
export interface ComponentShardWeights extends ShardWeights {
  readonly jobOverheadSeconds: number;
  readonly modelErrorSeconds: number;
}
export function listComponentFiles(): string[];
export function loadComponentWeights(text?: string): ComponentShardWeights;
export function componentShardFiles(
  index: number,
  count: number,
  files?: readonly string[]
): string[];
export function loadShardWeights(text?: string): ShardWeights;
export function listAcceptanceFiles(directory?: URL): string[];
export function splitAcceptanceFiles(
  files: readonly string[],
  count: number,
  weights?: ShardWeights
): FileShard[];
export function acceptanceShardFiles(
  index: number,
  count: number,
  files?: readonly string[]
): string[];
