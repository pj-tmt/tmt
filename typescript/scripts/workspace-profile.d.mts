export interface ProfileState {
  phase: string;
  phases: { name: string; seconds: number }[];
  harnesses: Harness[];
  active: Harness | null;
}
export interface Harness {
  name: string;
  executable: string | null;
  kind: string;
  phase: string;
  startSeconds: number;
  complete: boolean;
  endSeconds?: number;
  wallSeconds?: number;
  passed?: number;
  failed?: number;
  ignored?: number;
  reportedSeconds?: number;
}
export function parseStat(
  text: string,
  ticksPerSecond: number
): {
  parent: number;
  startTicks: number;
  selfSeconds: number;
  waitedSeconds: number;
};
export function parseCpu(text: string): {
  totalTicks: number;
  idleTicks: number;
  ioWaitTicks: number;
  running: number;
};
export function observeLine(state: ProfileState, line: string, seconds: number): void;
export function treeBytes(root: string): {
  logicalBytes: number;
  allocatedBytes: number;
  files: number;
  followsSymlinks: boolean;
} | null;
export function measureCommand(
  output: string,
  argv: string[],
  options?: { intervalMs?: number; stdout?: { write(chunk: Uint8Array): unknown } }
): Promise<number>;

export function readResourceSample(
  rootPid: number,
  target: string,
  ticks: number,
  phase: string,
  procRoot?: string
): {
  unavailable?: string;
  cpu?: ReturnType<typeof parseCpu>;
  harnessProcesses: {
    pid: number;
    executable: string;
    startTicks: number;
    sampledInclusiveCpuSeconds: number;
  }[];
};
