import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { PINNED_ADAPTER_TESTS, runAdapterTests } from '../../scripts/e2e-adapter-tests.mjs';
import { writeExecutable } from '../support/executable-fixture.mjs';

const roots: string[] = [];
afterEach(() => roots.splice(0).forEach((root) => rmSync(root, { recursive: true, force: true })));
const retired = 'tmux::io_tests::malformed_explicit_environment_and_scopes_never_spawn';
const ignored =
  'tmux::server_id_tests::server_id_concurrent_callers_adopt_one_id_on_a_fresh_server';
const inventory = [...PINNED_ADAPTER_TESTS, retired, ignored];
const good = 'test result: ok. 1 passed; 0 failed; 0 ignored; 99 filtered out; finished in 0.01s\n';
const result = (stdout: string) => ({ status: 0, signal: null, stdout, stderr: '' });
function port(list = inventory) {
  return vi.fn((_binary: string, args: string[]) =>
    result(args[0] === '--list' ? list.map((name) => `${name}: test`).join('\n') + '\n' : good)
  );
}

describe('the pinned adapter environment slice', () => {
  it('selects every approved case once, excluding scripted and ignored tmux cases', () => {
    const execute = port();
    const output = vi.fn();
    const completed = runAdapterTests({ execute, output });
    expect(execute.mock.calls[0][1]).toEqual(['--list', '--format', 'terse']);
    expect(execute.mock.calls.slice(1).map(([, args]) => args)).toEqual(
      PINNED_ADAPTER_TESTS.map((name) => [
        '--exact',
        '--include-ignored',
        '--format',
        'pretty',
        name,
      ])
    );
    expect(completed.tests).toBe(PINNED_ADAPTER_TESTS.length);
    expect(execute.mock.calls.flatMap(([, args]) => args)).not.toContain(retired);
    expect(execute.mock.calls.flatMap(([, args]) => args)).not.toContain(ignored);
    expect(output).toHaveBeenCalledWith(
      1,
      expect.stringContaining('Adapter environment execution seconds:')
    );
  });
  it('matches the grep list of two active real-ps forwarding tests', () => {
    const sourceRoot = fileURLToPath(
      new URL('../../../rust/crates/tmt-adapters/src/', import.meta.url)
    );
    const grep = ['process/process_info.rs', 'process/ancestry.rs'].flatMap((file) => {
      const source = readFileSync(path.join(sourceRoot, file), 'utf8');
      return [
        ...source.matchAll(
          /#\[test\]\s+fn (\w+)\([^)]*\)\s*\{([\s\S]*?)(?=\n    #\[test\]|\n\}\s*$)/g
        ),
      ]
        .filter(
          (m) =>
            /struct Ps(?:Only)?;/.test(m[2]) && m[2].includes('UnixCommandRunner.execute(request)')
        )
        .map((m) => `${file.replace(/\.rs$/, '').replaceAll('/', '::')}::tests::${m[1]}`);
    });
    expect(PINNED_ADAPTER_TESTS.filter((name) => name.startsWith('process::')).sort()).toEqual(
      grep.sort()
    );
    // The list is the sole selection owner if Core later approves another case.
    for (const name of PINNED_ADAPTER_TESTS) {
      const pieces = name.split('::');
      const fn = pieces.pop()!;
      if (pieces.at(-1) === 'tests') pieces.pop();
      const source = readFileSync(path.join(sourceRoot, pieces.join('/') + '.rs'), 'utf8');
      expect(source).toContain(`fn ${fn}(`);
    }
  });
  it.each(['missing', 'renamed', 'duplicate'])(
    'refuses %s inventory before executing tests',
    (mode) => {
      const list =
        mode === 'duplicate'
          ? [...inventory, PINNED_ADAPTER_TESTS[0]]
          : mode === 'renamed'
            ? [PINNED_ADAPTER_TESTS[0] + '_renamed', ...inventory.slice(1)]
            : inventory.slice(1);
      const execute = port(list);
      expect(() => runAdapterTests({ execute, output: vi.fn() })).toThrow(
        'Expected exactly one listed adapter test'
      );
      expect(execute).toHaveBeenCalledTimes(1);
    }
  );
  it.each([
    '0 passed; 0 failed; 0 ignored',
    '0 passed; 0 failed; 1 ignored',
    '2 passed; 0 failed; 0 ignored',
  ])('refuses exit-zero result %s', (counts) => {
    const execute = port();
    execute
      .mockImplementationOnce(() => result(inventory.map((name) => `${name}: test`).join('\n')))
      .mockImplementationOnce(() => result(`test result: ok. ${counts}; 99 filtered out\n`));
    expect(() => runAdapterTests({ execute, output: vi.fn() })).toThrow('did not run exactly once');
    expect(execute).toHaveBeenCalledTimes(2);
  });
  it('propagates a failing binary and exposes its captured output', () => {
    const execute = port();
    execute
      .mockImplementationOnce(() => result(inventory.map((name) => `${name}: test`).join('\n')))
      .mockImplementationOnce(() => ({ ...result('FAIL retained ps case\n'), status: 7 }));
    const output = vi.fn();
    expect(() => runAdapterTests({ execute, output })).toThrow('failed: 7');
    expect(output).toHaveBeenCalledWith(1, 'FAIL retained ps case\n');
    expect(execute).toHaveBeenCalledTimes(2);
  });
  it('runs the existing executable through real child processes with exact filters', () => {
    const root = mkdtempSync(path.join(tmpdir(), 'e2e-adapter-slice-'));
    roots.push(root);
    const binary = path.join(root, 'adapter-tests');
    writeExecutable(
      binary,
      `#!/usr/bin/env node\nconst args = process.argv.slice(2);\nconst names = ${JSON.stringify(inventory)};\nif (args[0] === '--list') process.stdout.write(names.map(n => n + ': test').join('\\n') + '\\n');\nelse {\n if (args[0] !== '--exact' || !${JSON.stringify(PINNED_ADAPTER_TESTS)}.includes(args.at(-1))) process.exit(9);\n process.stdout.write(${JSON.stringify(good)});\n}\n`,
      0o755
    );
    const calls: string[][] = [];
    const execute = (
      file: string,
      args: string[],
      options: { encoding: 'utf8'; timeout: number; maxBuffer: number }
    ) => {
      calls.push(args);
      return spawnSync(file, args, { ...options, encoding: 'utf8' });
    };
    expect(runAdapterTests({ binary, execute, output: vi.fn() }).tests).toBe(
      PINNED_ADAPTER_TESTS.length
    );
    expect(calls.slice(1).map((args) => args.at(-1))).toEqual(PINNED_ADAPTER_TESTS);
  });
});
