import { expect } from 'vite-plus/test';
import { parseWholeStdout, runCli, type Sandbox } from './cli-process.js';

export async function cli(sandbox: Sandbox, args: string[]) {
  const result = await runCli(sandbox, [...args, '--json']);
  expect(result.status, result.stdout + result.stderr).toBe(0);
  return parseWholeStdout(result);
}
