export function parseLibtestList(text: string): string[];
export function cargoInventory(
  artifacts: unknown[],
  list: (binary: string, ignored: boolean) => string
): unknown[];
export function verifyNextestInventory(
  cargo: unknown[],
  full: unknown,
  partitions: unknown[]
): {
  harnesses: string[];
  tests: { id: string[]; ignored: boolean }[];
  runnable: number;
  ignored: number;
  partitionCounts: number[];
};
