export interface WriteExecutableOptions {
  /** How long the writer may take before it is killed and the stall reported. Default 60 s. */
  timeoutMs?: number;
  /** A script run in place of the real writer, with `--write FILE MODE`; for tests of the stall report. */
  writer?: string;
}
export function writeExecutable(
  file: string,
  contents: string | Uint8Array,
  mode?: number,
  options?: WriteExecutableOptions
): void;
