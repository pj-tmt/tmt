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
