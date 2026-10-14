export function verifyColabApp(options: {
  executable: string;
  /** Fixture executable argument prefix; product callers leave it empty. */
  args?: readonly string[];
  version: string;
  expectedApp?: string;
  notices: string;
  tmtExecutable?: string;
}): Promise<void>;
export function appEntries(text?: string): { html: string[]; text: string[] };
export function expectedFiles(directory: string): Map<string, Buffer>;

/** Require the embedded hosted inventory to match the declaration and archive notices. */
export function verifyColabHosting(
  options: { executable: string; cwd: string; env: NodeJS.ProcessEnv; notices: string },
  run?: (
    executable: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; maxBuffer?: number }
  ) => string
): void;
