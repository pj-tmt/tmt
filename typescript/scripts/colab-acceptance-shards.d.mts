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
