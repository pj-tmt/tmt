import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { describe, expect, it } from 'vitest';
import { expectError, fileSnapshot, runCli, withSandbox } from '../support/cli-process.js';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';

// The shared selector validates the repository native build before allocating
// each sandbox. Explicit descriptors remain available for moved executables.

describe('native grammar process contract', () => {
  it('rejects misplaced run options before binding, configuration or storage effects', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(sandbox.globalConfig, '{broken configuration');
      const before = fileSnapshot(sandbox.root);
      const tmuxBaseline = readFileSync(tripwire, 'utf8');
      for (const option of ['-s', '--save', '--resume', '--help']) {
        const result = await runCli(sandbox, ['run', 'Alice', option]);
        expect(result.status).toBe(1);
        expect(result.stderr).toContain('TMT options go before the name');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
        expect(existsSync(sandbox.database)).toBe(false);
        expect(readFileSync(tripwire, 'utf8')).toBe(tmuxBaseline);
      }
    });
  });

  it('queries completion without configuration, tmux probing or storage initialization', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(sandbox.globalConfig, '{broken configuration');
      const before = fileSnapshot(sandbox.root);
      const tmuxBaseline = readFileSync(tripwire, 'utf8');
      const empty = await runCli(sandbox, ['__complete', '--', 'run', '']);
      expect(empty.status, empty.stderr).toBe(0);
      expect(empty.stdout).toBe('identities\n');
      expect(empty.stderr).toBe('');
      expect(fileSnapshot(sandbox.root)).toEqual(before);
      expect(existsSync(sandbox.database)).toBe(false);
      for (const name of ['Zulu', 'Alice Example']) {
        const created = await runCli(sandbox, ['identity', 'create', name, '--json']);
        expect(created.status, created.stderr).toBe(0);
      }
      const databaseBefore = readFileSync(sandbox.database);
      for (const args of [
        ['run', ''],
        ['this', ''],
        ['rm', ''],
        ['talk', 'Bob', 'hello', '--identity', ''],
      ]) {
        const result = await runCli(sandbox, ['__complete', '--', ...args]);
        expect(result.status, result.stderr).toBe(0);
        expect(result.stdout).toBe('identities\nAlice Example\nZulu\n');
        expect(result.stderr).toBe('');
      }
      const resume = await runCli(sandbox, ['__complete', '--', 'run', '--resume', '']);
      expect(resume.stdout).toBe('identities\n');
      const command = await runCli(sandbox, [
        '__complete',
        '--',
        'run',
        'Alice Example',
        'claude',
        '--identity=literal',
      ]);
      expect(command.status, command.stderr).toBe(0);
      expect(command.stdout).toBe('command\n2\n');
      expect(command.stderr).toBe('');
      expect(readFileSync(sandbox.database)).toEqual(databaseBefore);
      expect(readFileSync(tripwire, 'utf8')).toBe(tmuxBaseline);
    });
  });

  it('prints scoped help without loading configuration, touching tmux or creating state', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(sandbox.globalConfig, '{broken configuration');
      const before = fileSnapshot(sandbox.root);
      const tmuxBaseline = readFileSync(tripwire, 'utf8');
      for (const command of [
        ['office'],
        ['office', 'board'],
        ['office', 'block', 'apply'],
        ['talk'],
        ['x', 'listen'],
        ['config', 'set'],
        ['name'],
      ]) {
        for (const argv of [
          [...command, '-h'],
          [...command, '--help'],
          ['help', ...command],
        ]) {
          const result = await runCli(sandbox, argv);
          expect(result.status, result.stderr).toBe(0);
          expect(result.stderr).toBe('');
          expect(result.stdout).toContain(`Usage: tmt ${command.join(' ')}`);
          expect(result.stdout).toContain('--help');
          expect(fileSnapshot(sandbox.root)).toEqual(before);
          expect(existsSync(sandbox.database)).toBe(false);
          expect(readFileSync(tripwire, 'utf8')).toBe(tmuxBaseline);
        }
      }
    });
  });

  it('generates valid shells without offering rejected or unrelated options', async () => {
    await withSandbox(async (sandbox) => {
      for (const shell of ['bash', 'zsh']) {
        const completion = await runCli(sandbox, ['completion', shell]);
        expect(completion.status).toBe(0);
        expect(completion.stderr).toBe('');
        expect(completion.stdout).toContain('--save');
        expect(completion.stdout).toContain('marked');
        expect(completion.stdout).not.toContain('--wait');
        expect(completion.stdout).not.toContain('--team');
        expect(completion.stdout).not.toContain('--config');
        expect(completion.stdout).not.toContain('--verbose');
        expect(completion.stdout).not.toContain('--debug');
        execFileSync(shell, ['-n', '-c', completion.stdout], {
          env: sandbox.env,
          cwd: sandbox.cwd,
          timeout: 5000,
          maxBuffer: 1024 * 1024,
        });
        if (shell === 'bash') {
          const executable = [sandbox.cli.executable, ...sandbox.cli.args]
            .map((value) => `'${value.replaceAll("'", "'\\''")}'`)
            .join(' ');
          const probe = `${completion.stdout}\ntmt() { ${executable} "$@"; }\nCOMP_WORDS=(tmt x ackall --)\nCOMP_CWORD=3\n_tmt tmt -- ackall\nprintf '%s\\n' "\${COMPREPLY[@]}"`;
          const candidates = execFileSync(shell, ['-c', probe], {
            env: sandbox.env,
            cwd: sandbox.cwd,
            timeout: 5000,
            maxBuffer: 1024 * 1024,
            encoding: 'utf8',
          })
            .trim()
            .split('\n');
          expect(candidates).toContain('--identity');
          expect(candidates).toContain('--json');
          expect(candidates).not.toContain('--after');
          expect(candidates).not.toContain('--limit');
          expect(candidates).not.toContain('--force');
        }
      }
    });
  });
  it('prints version and grammar-backed help without bootstrapping storage', async () => {
    await withSandbox(async (sandbox) => {
      const before = fileSnapshot(sandbox.root);
      const version = await runCli(sandbox, ['--version']);
      expect(version.status).toBe(0);
      expect(version.stdout).toBe('5.0.0-alpha.8\n');
      expect(version.stderr).toBe('');
      const help = await runCli(sandbox, ['help']);
      expect(help.status).toBe(0);
      expect(help.stderr).toBe('');
      expect(help.stdout).toContain('TMT native alpha');
      expect(help.stdout).toContain('# Update a managed installation\n  tmt upgrade');
      for (const command of [
        'talk',
        'reply',
        'result',
        'check',
        'role',
        'preamble',
        'notes',
        'x',
        'install',
        'room',
        'identity',
      ]) {
        expect(help.stdout).toMatch(new RegExp(`^  ${command}\\s`, 'm'));
      }
      expect(help.stdout).toContain('Manage identity records, metadata and self-reported status');
      expect(help.stdout).toContain('Access saved identity notes');
      expect(help.stdout).toContain('temporary unless saved');
      expect(help.stdout).toContain('rm');
      expect(help.stdout).toContain('remove role/preamble');
      expect(help.stdout).toContain('keep pane/exchanges');
      expect(help.stdout).not.toContain('--wait');
      expect(help.stdout).not.toContain('--verbose');
      expect(help.stdout).not.toContain('--debug');
      expect(fileSnapshot(sandbox.root)).toEqual(before);
    });
  });

  it('rejects invalid options before root help or version with JSON diagnostics', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      const before = fileSnapshot(sandbox.root);
      const tmuxBaseline = readFileSync(tripwire, 'utf8');
      for (const args of [
        ['--json', 'help', '--timeout', '1s'],
        ['--json', '--help', '--timeout', '1s'],
        ['--json', '--version', '--timeout', '1s'],
      ]) {
        const result = await runCli(sandbox, args);
        expect(result.status).toBe(1);
        expectError(result, 'USAGE_ERROR');
        expect(result.stderr).toBe('');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
        expect(existsSync(sandbox.database)).toBe(false);
        expect(readFileSync(tripwire, 'utf8')).toBe(tmuxBaseline);
      }
    });
  });

  it('runs no command for a host driver, only help and the version', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      const before = fileSnapshot(sandbox.root);
      const tmuxBaseline = readFileSync(tripwire, 'utf8');
      // Every host-driver call carries this; a driver that runs tmt changes nothing.
      sandbox.env.TMT_DRIVER_CALL = '1';
      for (const args of [
        ['ls', '--json'],
        ['room', 'create', 'Nested', '--json'],
        ['identity', 'create', 'Nested', '--json'],
      ]) {
        const result = await runCli(sandbox, args);
        expect(result.status, args.join(' ')).toBe(1);
        expectError(result, 'DRIVER_CALL_REFUSED');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
        expect(existsSync(sandbox.database)).toBe(false);
        expect(readFileSync(tripwire, 'utf8')).toBe(tmuxBaseline);
      }
      const version = await runCli(sandbox, ['--version']);
      expect(version.status, version.stderr).toBe(0);
      const help = await runCli(sandbox, ['help', 'ls']);
      expect(help.status, help.stderr).toBe(0);
      delete sandbox.env.TMT_DRIVER_CALL;
      const listed = await runCli(sandbox, ['ls', '--json']);
      expect(listed.status, listed.stderr).toBe(0);
    });
  });

  it('rejects ignored options in every placement before storage or tmux effects', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      const before = fileSnapshot(sandbox.root);
      const tmuxBaseline = readFileSync(tripwire, 'utf8');
      for (const args of [
        ['list', '--config', '/tmp/ignored.json', '--json'],
        ['--config', '/tmp/ignored.json', 'list', '--json'],
        ['list', '--timeout', '1s', '--json'],
        ['--timeout', '1s', 'list', '--json'],
        ['role', 'show', '--timeout', '1s', '--json'],
        ['--timeout', '1s', 'role', 'show', '--json'],
      ]) {
        const result = await runCli(sandbox, args);
        expect(result.status).toBe(1);
        expectError(result, 'USAGE_ERROR');
        expect(result.stderr).toBe('');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
        expect(existsSync(sandbox.database)).toBe(false);
        expect(readFileSync(tripwire, 'utf8')).toBe(tmuxBaseline);
      }
    });
  });

  it('gives scoped corrective usage for a rejected placement option in both output modes', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      const before = fileSnapshot(sandbox.root);
      const tmuxBaseline = readFileSync(tripwire, 'utf8');
      const human = await runCli(sandbox, ['role', 'show', '--timeout', '1s']);
      expect(human.status).toBe(1);
      expect(human.stdout).toBe('');
      expect(human.stderr).toContain('Usage: tmt role show');
      expect(human.stderr).toContain('tmt help role show');

      const machine = await runCli(sandbox, ['role', 'show', '--timeout', '1s', '--json']);
      expect(machine.status).toBe(1);
      expect(machine.stderr).toBe('');
      const error = expectError(machine, 'USAGE_ERROR');
      expect(error).toMatchObject({
        error: { message: expect.stringContaining('Usage: tmt role show') },
      });
      expect(error).toMatchObject({
        error: { message: expect.stringContaining('tmt help role show') },
      });
      expect(fileSnapshot(sandbox.root)).toEqual(before);
      expect(existsSync(sandbox.database)).toBe(false);
      expect(readFileSync(tripwire, 'utf8')).toBe(tmuxBaseline);
    });
  });

  it('rejects former no-op output flags before effects without confusing version or literal values', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      const before = fileSnapshot(sandbox.root);
      const tmuxBaseline = readFileSync(tripwire, 'utf8');
      for (const flag of ['--verbose', '-v', '--debug']) {
        for (const args of [
          [flag, 'ls'],
          ['ls', flag],
          ['help', flag],
          ['--version', flag],
          ['identity', 'create', 'Agent', flag],
          ['send', 'peer', 'message', '--detach', flag],
        ]) {
          for (const argv of [args, ['--json', ...args], [...args, '--json']]) {
            const result = await runCli(sandbox, argv);
            expect(result.status).toBe(1);
            if (argv.includes('--json')) {
              expectError(result, 'USAGE_ERROR');
              expect(result.stdout).toContain(flag);
              expect(result.stderr).toBe('');
            } else {
              expect(result.stdout).toBe('');
              expect(result.stderr).toContain(flag);
            }
            expect(fileSnapshot(sandbox.root)).toEqual(before);
            expect(readFileSync(tripwire, 'utf8')).toBe(tmuxBaseline);
          }
        }
      }
      const literal = await runCli(sandbox, ['identity', 'create', '--json', '--', '--debug']);
      expect(literal.status).toBe(0);
      expect(JSON.parse(literal.stdout).identity.name).toBe('--debug');
      const version = await runCli(sandbox, ['--version']);
      expect(version.status).toBe(0);
      expect(version.stdout).toBe('5.0.0-alpha.8\n');
    });
  });

  it('rejects JSON mode for text-only commands before side effects', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      const before = fileSnapshot(sandbox.root);
      const tmuxBaseline = readFileSync(tripwire, 'utf8');
      for (const args of [
        ['help', '--json'],
        ['--json', '--help'],
        ['--version', '--json'],
        ['--json', '--version'],
        ['completion', 'bash', '--json'],
        ['learn', '--json'],
      ]) {
        const result = await runCli(sandbox, args);
        expect(result.status).toBe(1);
        expectError(result, 'JSON_UNSUPPORTED');
        expect(result.stderr).toBe('');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
        expect(existsSync(sandbox.database)).toBe(false);
        expect(readFileSync(tripwire, 'utf8')).toBe(tmuxBaseline);
      }
    });
  });

  it('rejects unmanaged upgrades before effects and supports structured alias failures', async () => {
    await withSandbox(async (sandbox) => {
      const before = fileSnapshot(sandbox.root);
      const result = await runCli(sandbox, ['upgrade']);
      expect(result.status).toBe(1);
      expect(result.stdout).toBe('');
      expect(result.stderr).toContain('not a managed native installation');
      const alias = await runCli(sandbox, ['update', '--json']);
      expect(alias.status).toBe(1);
      expectError(alias, 'NATIVE_UPGRADE_FAILED');
      expect(alias.stderr).toBe('');
      expect(fileSnapshot(sandbox.root)).toEqual(before);
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });

  it('keeps diagnostic mode out of literal values and honors later real flags', async () => {
    await withSandbox(async (sandbox) => {
      const before = fileSnapshot(sandbox.root);
      for (const args of [
        ['role', 'set', 'body', '--file=--json'],
        ['list', '--', '--json', 'extra'],
        ['learn', '--config', '--json'],
      ]) {
        const result = await runCli(sandbox, args);
        expect(result.status).toBe(1);
        expect(result.stdout).toBe('');
        expect(result.stderr).not.toBe('');
      }
      for (const args of [
        ['list', '--identity', 'caller', '--json'],
        ['--identity', 'caller', 'talk', 'peer', 'hello', '--json'],
        ['name', '--json'],
        ['talk', 'peer', '--identity', '--json', 'hello'],
        ['talk', 'peer', '--identity', '--json', 'hello', '--nope'],
        ['no-command', 'talk', 'peer', '--identity', '--json', '--nope'],
      ]) {
        const result = await runCli(sandbox, args);
        expect(result.status).toBe(1);
        expectError(result, 'USAGE_ERROR');
      }
      expect(fileSnapshot(sandbox.root)).toEqual(before);
    });
  });

  it('runs the selected native executable from a path containing spaces and quotes', async () => {
    await withSandbox(async (sandbox) => {
      expect(sandbox.cli.args).toEqual([]);
      const directory = path.join(sandbox.root, "native build's files");
      mkdirSync(directory);
      const executable = path.join(directory, 'tmt preview');
      copyFileSync(sandbox.cli.executable, executable);
      const result = await runCli({ ...sandbox, cli: { executable, args: [] } }, ['--version']);
      expect(result.status).toBe(0);
      expect(result.stdout).toBe('5.0.0-alpha.8\n');
      expect(result.stderr).toBe('');
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });
});
