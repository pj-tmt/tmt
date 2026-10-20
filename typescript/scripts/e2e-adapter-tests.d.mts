export const PINNED_ADAPTER_TESTS: readonly string[];
export function runAdapterTests(options?: {
  binary?: string;
  execute?: (
    binary: string,
    args: string[],
    options: {
      encoding: 'utf8';
      stdio: string[];
      timeout: number;
      maxBuffer: number;
    }
  ) => {
    status: number | null;
    signal: string | null;
    stdout: string;
    stderr: string;
    error?: Error;
  };
  output?: (stream: number, text: string) => void;
}): { tests: number; seconds: number };
