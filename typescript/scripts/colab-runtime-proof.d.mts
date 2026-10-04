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
