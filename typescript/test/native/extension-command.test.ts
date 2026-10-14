import { existsSync, mkdirSync, realpathSync, symlinkSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';

function extension(sandbox: Sandbox, name: string, script: string): string {
  const directory = path.join(sandbox.root, 'extension bin');
  mkdirSync(directory, { recursive: true });
  const file = path.join(directory, `tmt-${name}`);
  writeExecutable(file, `#!/bin/sh\n${script}\n`, 0o700);
  sandbox.env.PATH = `${directory}${path.delimiter}${sandbox.env.PATH ?? ''}`;
  return file;
}

describe('PATH extension command contract', () => {
  it('dispatches an installed retired Office executable through the generic PATH boundary', async () => {
    await withSandbox(async (sandbox) => {
      extension(sandbox, 'office', 'printf "%s\\n" "$@"; exit 23');
      const result = await runCli(sandbox, ['office', 'status', '--json']);
      expect(result.status).toBe(23);
      expect(result.stdout).toBe('status\n--json\n');
      expect(result.stderr).toBe('');
      expect((await runCli(sandbox, ['help', 'office'])).stdout).toBe('--help\n');
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });

  it('recovers rejection mode from the Core prefix without reading or executing the tail', async () => {
    await withSandbox(async (sandbox) => {
      const marker = path.join(sandbox.root, 'extension-executed');
      sandbox.env.TMT_EXTENSION_MARKER = marker;
      extension(sandbox, 'example', ': > "$TMT_EXTENSION_MARKER"; printf "%s\\n" "$@"');
      for (const prefix of [['--json'], ['--json', '--timeout', '1']]) {
        const result = await runCli(sandbox, [...prefix, 'example', 'status']);
        expect(result.status).toBe(1);
        expect(result.stderr).toBe('');
        expect(JSON.parse(result.stdout)).toEqual({
          error: {
            code: 'USAGE_ERROR',
            message: 'Put options after the extension name: tmt <name> [options].',
          },
        });
        expect(existsSync(marker)).toBe(false);
      }
      for (const prefix of [['--timeout', '1'], ['--timeout=--json']]) {
        const result = await runCli(sandbox, [...prefix, 'example', '--json']);
        expect(result.status).toBe(1);
        expect(result.stdout).toBe('');
        expect(result.stderr).toContain('Put options after the extension name');
        expect(existsSync(marker)).toBe(false);
      }
      const result = await runCli(sandbox, ['example', '--json', '--', 'literal']);
      expect(result.status).toBe(0);
      expect(result.stdout).toBe('--json\n--\nliteral\n');
      expect(result.stderr).toBe('');
      expect(existsSync(marker)).toBe(true);
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });

  it('keeps hidden suggestions out of human and JSON errors while preserving public suggestions', async () => {
    await withSandbox(async (sandbox) => {
      for (const json of [false, true]) {
        const prefix = json ? ['--json'] : [];
        const hidden = await runCli(sandbox, [...prefix, '__native-instal']);
        expect(hidden.status).toBe(1);
        const message = json ? JSON.parse(hidden.stdout).error.message : hidden.stderr;
        expect(message).toContain("'__native-instal'");
        expect(message).not.toContain("'__native-install'");
        if (json) {
          expect(hidden.stderr).toBe('');
          expect(JSON.parse(hidden.stdout)).toEqual({
            error: { code: 'USAGE_ERROR', message },
          });
          expect(message).not.toContain('Run tmt help');
        } else {
          expect(hidden.stdout).toBe('');
          expect(message).toContain('Run tmt help to see the commands and options');
          expect(message.match(/hint:/g)).toHaveLength(1);
          expect(message).not.toContain('For more information');
        }
        const mixed = await runCli(sandbox, [...prefix, 'extension install']);
        expect(mixed.status).toBe(1);
        const mixedMessage = json ? JSON.parse(mixed.stdout).error.message : mixed.stderr;
        expect(mixedMessage).toContain("'extension'");
        expect(mixedMessage).not.toContain('__native-install');
        const publicResult = await runCli(sandbox, [...prefix, 'extension', 'uninstal']);
        expect(publicResult.status).toBe(1);
        const publicMessage = json
          ? JSON.parse(publicResult.stdout).error.message
          : publicResult.stderr;
        expect(publicMessage).toContain("'uninstall'");
      }
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });

  it('forwards nested help and alias tails verbatim and keeps missing-extension error mode', async () => {
    await withSandbox(async (sandbox) => {
      sandbox.env.TMT_AGENT = '1';
      sandbox.env.BROWSER = '/usr/bin/true';
      const file = extension(sandbox, 'example', 'printf "%s\\n" "$@"; exit 23');
      symlinkSync(file, path.join(path.dirname(file), 'tmt-alias'));
      for (const name of ['example', 'alias']) {
        for (const tail of [[], ['child'], ['child', '--json', '--', 'two words', '']]) {
          const result = await runCli(sandbox, ['help', name, ...tail]);
          expect(result.status).toBe(23);
          expect(result.stdout).toBe([...tail, '--help'].join('\n') + '\n');
          expect(result.stderr).toBe('');
        }
      }
      const absent = await runCli(sandbox, ['help', 'no-installed-extension', 'child', '--json']);
      expect(absent.status).toBe(1);
      expect(absent.stderr).toBe('');
      expect(JSON.parse(absent.stdout)).toMatchObject({
        error: {
          code: 'USAGE_ERROR',
          message: expect.stringContaining("Unknown command 'no-installed-extension' after tmt"),
        },
      });
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });

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
      expect(misplaced.status).toBe(1);
      expect(misplaced.stderr).toBe('');
      expect(JSON.parse(misplaced.stdout)).toEqual({
        error: {
          code: 'USAGE_ERROR',
          message: 'Put options after the extension name: tmt <name> [options].',
        },
      });
      const absent = await runCli(sandbox, ['--json', 'no-installed-extension', '--dry-run']);
      expect(absent.status).toBe(1);
      expect(absent.stderr).toBe('');
      expect(JSON.parse(absent.stdout)).toMatchObject({
        error: { code: 'USAGE_ERROR', message: expect.stringContaining('unrecognized subcommand') },
      });
      for (const name of ['identity', 'ls', '__complete']) {
        extension(sandbox, name, 'printf "WRONG EXTENSION\\n"; exit 99');
      }
      expect((await runCli(sandbox, ['identity', '--help'])).stdout).not.toContain(
        'WRONG EXTENSION'
      );
      expect((await runCli(sandbox, ['ls', '--help'])).stdout).not.toContain('WRONG EXTENSION');
      expect((await runCli(sandbox, ['__complete', '--', 'run', ''])).stdout).toBe(
        'identities\nclaude\ncodex\n'
      );
      const help = await runCli(sandbox, ['help']);
      expect(help.stdout).toContain('\nExtensions:\n');
      expect(help.stdout).toMatch(/^ {2}example +\S/m);
      for (const name of ['identity', 'ls', '__complete']) {
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
      writeExecutable(broken, '#!/nonexistent-tmt-fixture-interpreter\n', 0o700);
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
