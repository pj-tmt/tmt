export interface TestCase {
  binary: string;
  name: string;
  ignored: boolean;
}
export function libtestCases(text: string): string[];
export function compareInventories(
  cargo: { executable: string; tests: string[]; ignored: string[] }[],
  nextest: {
    'rust-suites': Record<
      string,
      {
        'binary-path': string;
        'binary-id': string;
        testcases: Record<
          string,
          { ignored: boolean; 'filter-match': { status: string; reason?: string } }
        >;
      }
    >;
  }
): TestCase[];
export function verifyExecution(
  cases: TestCase[],
  log: string
): { passed: number; ignored: number; total: number };
export const NEXTEST_VERSION: string;
export const TOOLCHAIN: string;
export type NextestInventory = Parameters<typeof compareInventories>[1];
export interface WorkspaceManifest {
  schema: number;
  identity: Record<string, string>;
  partitions: number;
  archive: { bytes: number; sha256: string };
  cases: TestCase[];
}
export interface PartitionResult {
  schema: number;
  identity: Record<string, string>;
  archive: { bytes: number; sha256: string };
  index: number;
  partitions: number;
  exit: number;
  worldRemoved: boolean;
  cases: TestCase[];
  log: string;
}
export function partitionCases(cases: TestCase[], index: number, total: number): TestCase[];
export function admitPartition(
  cases: TestCase[],
  inventory: NextestInventory,
  index: number,
  total: number
): TestCase[];
export function aggregatePartitions(
  manifest: WorkspaceManifest,
  results: PartitionResult[],
  states: { producer: string; consumers: string }
): { passed: number; ignored: number; total: number };
export function admitArchive(
  manifest: WorkspaceManifest,
  bytes: Uint8Array,
  expectedIdentity: Record<string, string>
): WorkspaceManifest;
