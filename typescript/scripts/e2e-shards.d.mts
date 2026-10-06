export interface ShardWeights {
  readonly adapterTestsSeconds: number;
  readonly defaultSeconds: number;
  readonly files: Readonly<Record<string, number>>;
}

export interface FileShard {
  readonly files: string[];
  readonly seconds: number;
}

export const FULL_E2E_SHARDS: number;
export const RETIRED_E2E_FILES: readonly string[];
export function loadShardWeights(text?: string): ShardWeights;
export function listE2eFiles(directory?: URL): string[];
export function splitE2eFiles(
  files: readonly string[],
  count: number,
  weights?: ShardWeights
): FileShard[];
export function e2eShardFiles(
  scope: string,
  scopedFiles: readonly string[],
  files?: readonly string[]
): [string[], string[]];
