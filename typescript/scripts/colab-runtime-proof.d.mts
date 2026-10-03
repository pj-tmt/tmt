export function verifyColabApp(options: {
  executable: string;
  /** Fixture executable argument prefix; product callers leave it empty. */
  args?: readonly string[];
  version: string;
  expectedApp?: string;
  notices: string;
  tmtExecutable?: string;
}): Promise<void>;
