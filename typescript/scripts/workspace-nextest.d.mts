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
  producerIdentity: Record<string, string>;
  partitions: number;
  archive: { bytes: number; sha256: string };
  cases: TestCase[];
  runtimeFiles: string[];
}
export interface PartitionResult {
  schema: number;
  producerIdentity: Record<string, string>;
  executionIdentity: Record<string, string>;
  manifestSha256: string;
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
  states: { producer: string; consumers: string },
  selection?: PartitionSelection
): { passed: number; ignored: number; total: number };
export function admitArchive(
  manifest: WorkspaceManifest,
  bytes: Uint8Array,
  expectedIdentity: Record<string, string>
): WorkspaceManifest;

export function consumerEnvironment(cargo: string, base?: NodeJS.ProcessEnv): NodeJS.ProcessEnv;
export function admitMachineIdentity(
  identity: Record<string, string>,
  expectedRun: Record<string, string>
): Record<string, string>;

export function exampleExecutables(
  artifacts: {
    reason: string;
    profile: { test: boolean };
    target: { kind: string[] };
    executable: string | null;
  }[],
  targetDirectory: string
): string[];
export function runtimeArchiveConfig(files: string[]): string;
export function verifyRuntimeExecutables(files: string[], targetDirectory: string): void;

export interface ProducerReceipt {
  proof: {
    schema: number;
    producerIdentity: Record<string, string>;
    partitions: number;
    manifestSha256: string;
    archive: WorkspaceManifest['archive'];
  };
  closure: { id: string; digest: string };
  inventory: { id: string; digest: string };
}
export interface PartitionSelection {
  schema: number;
  producerIdentity: Record<string, string>;
  collectorIdentity: Record<string, string>;
  manifestSha256: string;
  partitions: {
    index: number;
    attempt: string;
    jobId: number | null;
    artifact: { id: string; name: string; digest: string } | null;
  }[];
}
export function resultArtifactName(
  index: number,
  identity: Record<string, string>,
  attempt: string
): string;
export function admitProducerReceipt(
  receipt: ProducerReceipt,
  expectedRun: Record<string, string>,
  producerResult: string
): ProducerReceipt;
export function admitManifest(receipt: ProducerReceipt, bytes: Uint8Array): WorkspaceManifest;
export function admitExecutionIdentity(
  producer: Record<string, string>,
  execution: Record<string, string>,
  collectorAttempt?: string
): Record<string, string>;
export function createRecoveryReader(
  repository: string,
  execute?: (program: string, args: string[], options: { timeoutMs: number }) => string,
  clock?: () => number
): (resource: string) => unknown;
export function planPartitionRecovery(
  receipt: ProducerReceipt,
  collector: Record<string, string>,
  states: { producer: string; consumers: string },
  read: (resource: string) => unknown,
  workflowHead?: string
): PartitionSelection;
