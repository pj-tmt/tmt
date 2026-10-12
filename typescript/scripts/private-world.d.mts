import type { SpawnSyncReturns } from 'node:child_process';

export interface PrivateWorld {
  readonly root: string;
  readonly home: string;
  readonly tmp: string;
  readonly config: string;
  readonly data: string;
  readonly state: string;
  readonly cache: string;
}
export interface WorldLeaks {
  /** Entries left in the private HOME and XDG roots. */
  readonly state: string[];
  /** Entries left in the private TMPDIR. */
  readonly temp: string[];
}
export const ROOT_LIMIT: number;
export function defaultParent(preferred?: string): string;
export function createWorld(parent?: string): PrivateWorld;
export function worldEnvironment(world: PrivateWorld, base?: NodeJS.ProcessEnv): NodeJS.ProcessEnv;
export function findLeaks(world: PrivateWorld): WorldLeaks;
export function runInPrivateWorld(options: {
  argv: readonly string[];
  enforce?: boolean;
  keep?: boolean;
  base?: NodeJS.ProcessEnv;
  parent?: string;
  spawn?: (
    command: string,
    args: string[],
    options: { env: NodeJS.ProcessEnv; stdio: 'inherit' }
  ) => Pick<SpawnSyncReturns<Buffer>, 'status' | 'signal' | 'error'>;
  log?: (line: string) => void;
}): { status: number; leaks: WorldLeaks; world: PrivateWorld };
