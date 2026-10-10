export const RELEASE_RECORD: string;
export const RECORD_LIMIT: number;
export const POINTER_LIMIT: number;
export interface IndexAsset {
  name: string;
  url: string;
  size: number;
  sha256: string;
}
export interface ReleaseRecord {
  schemaVersion: 1;
  product: string;
  version: string;
  tag: string;
  releaseId: number;
  sourceSha: string;
  manifest: IndexAsset;
  archives: Record<string, IndexAsset>;
}
export interface ChannelPointer {
  schemaVersion: 1;
  product: string;
  channel: 'alpha' | 'stable';
  version: string;
  tag: string;
  record: Omit<IndexAsset, 'name'>;
}
export interface RecordInput {
  product: string;
  tag: string;
  releaseId: number;
  sourceSha: string;
  directory: string;
}
export function validateReleaseRecord(record: unknown): ReleaseRecord;
export function parseReleaseRecord(bytes: Buffer | string): ReleaseRecord;
export function parseChannelPointer(bytes: Buffer | string): ChannelPointer;
export function createReleaseRecord(input: RecordInput): ReleaseRecord;
export function releaseRecordBytes(record: ReleaseRecord): Buffer;
export function verifyReleaseRecord(
  input: RecordInput & { recordBytes: Buffer | string }
): ReleaseRecord;

export interface IndexTip {
  sha: string;
  tree: string;
}
export interface IndexApi {
  readTip(): IndexTip;
  readFile(tip: string, path: string, limit: number): Buffer | null;
  createCommit(tip: IndexTip, entries: { path: string; bytes: Buffer }[]): string;
  advance(commit: string): 'updated' | 'race';
}
export interface IndexResult {
  changed: boolean;
  tip: string;
  path: string;
  attempts: number;
}
export function channelPointer(
  bytes: Buffer | string,
  options?: { bootstrap?: boolean }
): ChannelPointer;
export function updateReleaseIndex(input: {
  api: IndexApi;
  recordBytes: Buffer | string;
  bootstrap?: boolean;
}): IndexResult;
export function ghIndexApi(input: {
  repository: string;
  env?: NodeJS.ProcessEnv;
  spawn?: (
    command: string,
    args: readonly string[],
    options: object
  ) => { error?: Error; status: number | null; stdout: string; stderr: string };
}): IndexApi;
