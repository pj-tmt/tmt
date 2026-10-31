/** Capture at most eight 64 KiB synthetic Codex inputs and forward identical bytes/status. */
export function forwardCapturedHook(
  executable: string,
  args: string[],
  directory: string,
  input: AsyncIterable<Uint8Array>
): Promise<number>;
