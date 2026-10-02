import { chmodSync, existsSync, mkdirSync, realpathSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';

function extension(sandbox: Sandbox, name: string, script: string): string {
  const directory = path.join(sandbox.root, 'extension bin');
  mkdirSync(directory, { recursive: true });
  const file = path.join(directory, `tmt-${name}`);
  writeFileSync(file, `#!/bin/sh\n${script}\n`);
  chmodSync(file, 0o700);
  sandbox.env.PATH = `${directory}${path.delimiter}${sandbox.env.PATH ?? ''}`;
  return file;
}

describe('PATH extension command contract', () => {
  it('reserves mv and rename before PATH dispatch and preserves the identity UUID', async () => {
    await withSandbox(async (sandbox) => {
      for (const name of ['mv', 'rename']) {
        extension(sandbox, name, 'printf "WRONG EXTENSION\\n"; exit 99');
      }
      const created = await runCli(sandbox, ['identity', 'create', 'worker', '--json']);
      expect(created.status).toBe(0);
      const id = JSON.parse(created.stdout).identity.id;
      for (const [command, old, next] of [
        ['mv', 'worker', 'reviewer'],
        ['rename', 'reviewer', 'worker'],
      ]) {
        const renamed = await runCli(sandbox, [command!, old!, next!, '--json']);
        expect(renamed.status).toBe(0);
        const shown = await runCli(sandbox, ['identity', 'show', next!, '--json']);
        expect(JSON.parse(shown.stdout).identity).toMatchObject({ id, name: next });
        const help = await runCli(sandbox, ['help', command!]);
        expect(help.stdout).toContain('Usage: tmt mv ');
        expect(help.stdout).not.toContain('WRONG EXTENSION');
      }
      const help = await runCli(sandbox, ['help']);
      for (const name of ['mv', 'rename']) {
        expect(help.stdout).toMatch(
          new RegExp(`^ {2}tmt-${name} +ignored: reserved core command`, 'm')
        );
      }
    });
  });

  it('executes the exact tail, preserves stdin/output/exit and exposes the invoking executable', async () => {
    await withSandbox(async (sandbox) => {
      extension(
        sandbox,
        'example',
        'printf "%s\\n" "$TMT_EXECUTABLE" "$@"; /bin/cat; printf "extension error\\n" >&2; exit 23'
      );
      const result = await runCli(
        sandbox,
        ['example', '--json', '--help', 'two words', '', "a'quote"],
        { stdin: 'input bytes\n' }
      );
      expect(result.status).toBe(23);
      expect(result.stdout).toBe(
        `${realpathSync(sandbox.cli.executable)}\n--json\n--help\ntwo words\n\na'quote\ninput bytes\n`
      );
      expect(result.stderr).toBe('extension error\n');
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });

  it('forwards help, rejects root option placement and retains core/alias/hidden precedence', async () => {
    await withSandbox(async (sandbox) => {
      extension(sandbox, 'example', 'printf "%s\\n" "$@"');
      expect((await runCli(sandbox, ['help', 'example'])).stdout).toBe('--help\n');
      const misplaced = await runCli(sandbox, ['--json', 'example', 'x']);
      expect(misplaced.status).not.toBe(0);
      expect(misplaced.stderr).toContain('Put options after the extension name');
      const absent = await runCli(sandbox, ['--json', 'no-installed-extension', '--dry-run']);
      expect(absent.status).toBe(1);
      expect(absent.stderr).toBe('');
      expect(JSON.parse(absent.stdout)).toMatchObject({
        error: { code: 'USAGE_ERROR', message: expect.stringContaining('unrecognized subcommand') },
      });
      for (const name of ['office', 'ls', '__complete']) {
        extension(sandbox, name, 'printf "WRONG EXTENSION\\n"; exit 99');
      }
      expect((await runCli(sandbox, ['office', '--help'])).stdout).not.toContain('WRONG EXTENSION');
      expect((await runCli(sandbox, ['ls', '--help'])).stdout).not.toContain('WRONG EXTENSION');
      expect((await runCli(sandbox, ['__complete', '--', 'run', ''])).stdout).toBe(
        'identities\nclaude\ncodex\n'
      );
      const help = await runCli(sandbox, ['help']);
      expect(help.stdout).toContain('\nExtensions:\n');
      expect(help.stdout).toMatch(/^ {2}example +\S/m);
      for (const name of ['office', 'ls', '__complete']) {
        expect(help.stdout).toMatch(
          new RegExp(`^ {2}tmt-${name} +ignored: reserved core command`, 'm')
        );
      }
      const root = await runCli(sandbox, ['__complete', '--', 'exa']);
      expect(root.stdout).toBe('root\nexample\n');
      expect((await runCli(sandbox, ['exampl'])).stderr).toContain('example');
    });
  });

  it('delegates literal completion and falls back on nonzero, empty, timeout and bounded output', async () => {
    await withSandbox(async (sandbox) => {
      extension(
        sandbox,
        'example',
        `test "$1" = __complete && test "$2" = -- && test "$3" = "two words" && test "$4" = "" || exit 9; printf '%s\\n' 'two words' "quote's" '--option'`
      );
      expect((await runCli(sandbox, ['__complete', '--', 'example', 'two words', ''])).stdout).toBe(
        "candidates\ntwo words\nquote's\n--option\n"
      );
      for (const script of [
        'printf "not a success\\n"; exit 1',
        'exit 0',
        '/bin/sleep 10',
        'while :; do printf "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n"; done',
      ]) {
        extension(sandbox, 'example', script);
        expect((await runCli(sandbox, ['__complete', '--', 'example', ''])).stdout).toBe('files\n');
      }
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });

  it('keeps normal exec failure and signal semantics without interpreting invalid names', async () => {
    await withSandbox(async (sandbox) => {
      extension(sandbox, 'signal', 'kill -TERM $$');
      const signaled = await runCli(sandbox, ['signal']);
      expect(signaled.status).toBeNull();
      expect(signaled.signal).toBe('SIGTERM');
      const broken = extension(sandbox, 'broken', 'exit 0');
      writeFileSync(broken, '#!/nonexistent-tmt-fixture-interpreter\n');
      const result = await runCli(sandbox, ['broken']);
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('Could not execute extension');
      for (const name of ['Upper', 'a.b']) {
        extension(sandbox, name, 'printf "WRONG EXTENSION\\n"');
        const invalid = await runCli(sandbox, [name]);
        expect(invalid.status).not.toBe(0);
        expect(invalid.stdout).not.toContain('WRONG EXTENSION');
      }
    });
  });
});
