import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { fileSnapshot, runCli, withSandbox } from '../support/cli-process.js';

describe('consented provider setup and bounded hook boundary', () => {
  it.each(['claude', 'codex'])(
    '%s preserves user settings, requires consent, repairs a stale path and removes only owned hooks',
    async (name) => {
      await withSandbox(async (sandbox) => {
        const bin = path.join(sandbox.root, 'stable bin');
        fs.mkdirSync(bin);
        const launcher = path.join(bin, 'tmt');
        fs.symlinkSync(sandbox.cli.executable, launcher);
        sandbox.env.PATH = `${bin}${path.delimiter}${sandbox.env.PATH ?? ''}`;
        const provider = path.join(sandbox.home, `.${name}`);
        fs.mkdirSync(provider);
        const settings = path.join(provider, name === 'claude' ? 'settings.json' : 'hooks.json');
        const userHook = '{ "hooks" : [{ "type": "command", "command": "user-command" }] }';
        const original = `{\n "permissions": {"allow": ["Bash(git *)"]},\n "future": 1.000e+100,\n "hooks": {"SessionStart": [${userHook}]}\n}\n`;
        fs.writeFileSync(settings, original);
        const before = fileSnapshot(sandbox.root);
        // Guided setup has work to do here, so without consent it only refuses.
        const status = await runCli(sandbox, ['setup', '--json']);
        expect(status.status, status.stderr).toBe(1);
        expect(JSON.parse(status.stdout).error.code).toBe('SETUP_CONSENT_REQUIRED');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
        const refused = await runCli(sandbox, ['setup', name, '--json']);
        expect(refused.status).toBe(1);
        expect(JSON.parse(refused.stdout).error.code).toBe('SETUP_CONSENT_REQUIRED');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
        const apply = await runCli(sandbox, ['setup', name, '--yes', '--json']);
        expect(apply.status, apply.stderr).toBe(0);
        const report = JSON.parse(apply.stdout);
        expect(report).toMatchObject({ changed: true, launcher, settingsPath: settings });
        expect(fs.readFileSync(report.backup, 'utf8')).toBe(original);
        const installed = fs.readFileSync(settings, 'utf8');
        expect(installed).toContain(userHook);
        expect(installed).toContain('"future": 1.000e+100');
        expect(installed).toContain(`${launcher}' __hook ${name}`);
        expect(installed).not.toContain(fs.realpathSync(launcher));
        const record = path.join(sandbox.globalDir, 'setup-record.json');
        const recorded = { driver: name, settings, launcher };
        expect(JSON.parse(fs.readFileSync(record, 'utf8'))).toEqual({
          version: 1,
          hooks: [recorded],
        });
        const recordBytes = fs.readFileSync(record);
        const again = await runCli(sandbox, ['setup', name, '--yes', '--json']);
        expect(again.status, again.stderr).toBe(0);
        expect(JSON.parse(again.stdout)).toMatchObject({ changed: false, backup: null });
        expect(fs.readFileSync(settings, 'utf8')).toBe(installed);
        expect(fs.readFileSync(record)).toEqual(recordBytes);
        // Exact hooks written before the record existed are adopted.
        fs.rmSync(record);
        expect((await runCli(sandbox, ['setup', name, '--yes', '--json'])).status).toBe(0);
        expect(fs.readFileSync(settings, 'utf8')).toBe(installed);
        expect(fs.readFileSync(record)).toEqual(recordBytes);
        fs.writeFileSync(settings, installed.replaceAll(launcher, '/old/release/tmt'));
        expect((await runCli(sandbox, ['setup', name, '--yes', '--json'])).status).toBe(0);
        expect(fs.readFileSync(settings, 'utf8')).toBe(installed);
        const remove = await runCli(sandbox, ['setup', name, '--remove', '--yes', '--json']);
        expect(remove.status, remove.stderr).toBe(0);
        const removed = fs.readFileSync(settings, 'utf8');
        expect(removed).toContain(userHook);
        expect(removed).not.toContain('__hook');
        expect(JSON.parse(removed).permissions).toEqual(JSON.parse(original).permissions);
        expect(JSON.parse(fs.readFileSync(record, 'utf8'))).toEqual({ version: 1, hooks: [] });
        expect(fs.existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it.each(['claude', 'codex'])(
    '%s adds the usage hook only on request, keeps it on rerun and removes it exactly',
    async (name) => {
      await withSandbox(async (sandbox) => {
        const bin = path.join(sandbox.root, 'bin');
        fs.mkdirSync(bin);
        fs.symlinkSync(sandbox.cli.executable, path.join(bin, 'tmt'));
        sandbox.env.PATH = `${bin}${path.delimiter}${sandbox.env.PATH ?? ''}`;
        const provider = path.join(sandbox.home, `.${name}`);
        fs.mkdirSync(provider);
        const settings = path.join(provider, name === 'claude' ? 'settings.json' : 'hooks.json');
        const original = '{\n "permissions": {"allow": []}\n}\n';
        fs.writeFileSync(settings, original);
        const events = () => Object.keys(JSON.parse(fs.readFileSync(settings, 'utf8')).hooks ?? {});
        const setup = async (...flags: string[]) => {
          const result = await runCli(sandbox, ['setup', name, ...flags, '--yes', '--json']);
          expect(result.status, result.stderr).toBe(0);
          return JSON.parse(result.stdout);
        };

        // Off by default; the report says nothing about usage.
        expect(await setup()).not.toHaveProperty('usage');
        expect(events()).toEqual(['SessionStart', 'SessionEnd', 'UserPromptSubmit']);
        const lifecycle = fs.readFileSync(settings, 'utf8');
        const preview = await runCli(sandbox, ['setup', name, '--usage']);
        expect(preview.stdout).toContain(
          'SessionStart, SessionEnd, UserPromptSubmit and Stop (context usage) hooks'
        );
        expect(preview.stdout).toContain('no transcript content is stored');
        expect(fs.readFileSync(settings, 'utf8')).toBe(lifecycle);

        expect(await setup('--usage')).toMatchObject({ changed: true, usage: true });
        expect(events()).toEqual(['SessionStart', 'SessionEnd', 'UserPromptSubmit', 'Stop']);
        expect(fs.readFileSync(settings, 'utf8')).toContain(`__hook ${name}`);
        const withUsage = fs.readFileSync(settings, 'utf8');
        // A rerun without a choice keeps it.
        expect(await setup()).toMatchObject({ changed: false, usage: true });
        expect(fs.readFileSync(settings, 'utf8')).toBe(withUsage);

        const opted = await runCli(sandbox, ['setup', name, '--no-usage']);
        expect(opted.stdout).toContain(`Remove TMT-owned ${name} Stop (context usage) hook`);
        expect(await setup('--no-usage')).not.toHaveProperty('usage');
        expect(fs.readFileSync(settings, 'utf8')).toBe(lifecycle);

        await setup('--usage');
        expect(await setup('--remove')).not.toHaveProperty('usage');
        expect(fs.readFileSync(settings, 'utf8')).toBe(original);
        for (const conflict of [
          ['--usage', '--no-usage'],
          ['--remove', '--usage'],
        ]) {
          const refused = await runCli(sandbox, ['setup', name, ...conflict, '--yes', '--json']);
          expect(refused.status).not.toBe(0);
          expect(JSON.parse(refused.stdout).error.code).toBe('USAGE_ERROR');
        }
      });
    }
  );

  it('stops before changing settings when the setup record is invalid', async () => {
    await withSandbox(async (sandbox) => {
      const bin = path.join(sandbox.root, 'bin');
      fs.mkdirSync(bin);
      fs.symlinkSync(sandbox.cli.executable, path.join(bin, 'tmt'));
      sandbox.env.PATH = `${bin}${path.delimiter}${sandbox.env.PATH ?? ''}`;
      fs.mkdirSync(path.join(sandbox.home, '.claude'));
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      const record = path.join(sandbox.globalDir, 'setup-record.json');
      fs.writeFileSync(record, '{"version":9}');
      const before = fileSnapshot(sandbox.root);
      const result = await runCli(sandbox, ['setup', 'claude', '--yes', '--json']);
      expect(result.status).toBe(1);
      expect(JSON.parse(result.stdout).error.code).toBe('SETUP_ERROR');
      expect(fileSnapshot(sandbox.root)).toEqual(before);
    });
  });

  it('does not rewrite malformed settings', async () => {
    await withSandbox(async (sandbox) => {
      const settings = path.join(sandbox.home, '.claude', 'settings.json');
      fs.mkdirSync(path.dirname(settings));
      fs.writeFileSync(settings, '{invalid');
      const before = fileSnapshot(sandbox.root);
      const result = await runCli(sandbox, ['setup', 'claude', '--yes', '--json']);
      expect(result.status).toBe(1);
      expect(fileSnapshot(sandbox.root)).toEqual(before);
    });
  });

  it('honors CODEX_HOME without changing provider trust or config', async () => {
    await withSandbox(async (sandbox) => {
      const bin = path.join(sandbox.root, 'stable-bin');
      fs.mkdirSync(bin);
      fs.symlinkSync(sandbox.cli.executable, path.join(bin, 'tmt'));
      sandbox.env.PATH = `${bin}${path.delimiter}${sandbox.env.PATH ?? ''}`;
      const custom = path.join(sandbox.root, 'custom-codex');
      fs.mkdirSync(custom);
      sandbox.env.CODEX_HOME = custom;
      const config = 'model = "fixture"\n';
      fs.writeFileSync(path.join(custom, 'config.toml'), config);
      const applied = await runCli(sandbox, ['setup', 'codex', '--yes', '--json']);
      expect(applied.status, applied.stderr).toBe(0);
      expect(JSON.parse(applied.stdout).settingsPath).toBe(path.join(custom, 'hooks.json'));
      expect(fs.readFileSync(path.join(custom, 'config.toml'), 'utf8')).toBe(config);
      expect(fs.readdirSync(custom).sort()).toEqual([
        '.tmt-setup.lock',
        'config.toml',
        'hooks.json',
      ]);
      expect(fs.existsSync(path.join(sandbox.home, '.codex', 'hooks.json'))).toBe(false);
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  });

  it('silently ignores a valid hook with no resolvable caller pane', async () => {
    await withSandbox(async (sandbox) => {
      const bin = path.join(sandbox.root, 'no-pane');
      fs.mkdirSync(bin);
      fs.writeFileSync(path.join(bin, 'tmux'), '#!/bin/sh\nexit 1\n', { mode: 0o755 });
      sandbox.env.PATH = `${bin}${path.delimiter}${sandbox.env.PATH ?? ''}`;
      const before = fileSnapshot(sandbox.root);
      for (const event of [
        { hook_event_name: 'SessionStart', source: 'startup' },
        { hook_event_name: 'SessionEnd', reason: 'prompt_input_exit' },
      ]) {
        const result = await runCli(sandbox, ['__hook', 'claude'], {
          stdin: JSON.stringify({ ...event, session_id: 'outside-tmux' }),
        });
        expect(result).toMatchObject({ status: 0, stdout: '', stderr: '' });
      }
      expect(fileSnapshot(sandbox.root)).toEqual(before);
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  });

  it('fails open without context or storage on malformed, excessive or incomplete hook input', async () => {
    await withSandbox(async (sandbox) => {
      const before = fileSnapshot(sandbox.root);
      for (const input of ['{malformed', 'x'.repeat(65537)]) {
        const result = await runCli(sandbox, ['__hook', 'claude'], { stdin: input });
        expect(result.status, result.stderr).toBe(0);
        expect(result.stdout).toBe('');
        expect(result.stderr.trim().split('\n')).toHaveLength(1);
        expect(result.stderr).not.toContain(input);
      }
      const started = performance.now();
      const slow = await runCli(sandbox, ['__hook', 'claude'], {
        stdin: '{',
        closeStdin: false,
        deadlineMs: 5000,
      });
      expect(slow.status, slow.stderr).toBe(0);
      expect(slow.stdout).toBe('');
      expect(performance.now() - started).toBeLessThan(4500);
      expect(fileSnapshot(sandbox.root)).toEqual(before);
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  });

  it('fails open and terminates a stalled probe and its descendant', async () => {
    await withSandbox(async (sandbox) => {
      const bin = path.join(sandbox.root, 'probes');
      fs.mkdirSync(bin);
      const pidFile = path.join(sandbox.root, 'probe-child');
      fs.writeFileSync(
        path.join(bin, 'tmux'),
        '#!/bin/sh\n/bin/sleep 20 &\necho $! > "$TMT_TEST_PROBE_PID"\nwait\n',
        { mode: 0o755 }
      );
      sandbox.env.PATH = `${bin}${path.delimiter}${sandbox.env.PATH ?? ''}`;
      sandbox.env.TMT_TEST_PROBE_PID = pidFile;
      let child: number | undefined;
      const failures: unknown[] = [];
      try {
        const result = await runCli(sandbox, ['__hook', 'claude'], {
          stdin: JSON.stringify({
            hook_event_name: 'SessionStart',
            source: 'startup',
            session_id: 'fixture-session',
          }),
          deadlineMs: 5000,
        });
        child = Number(fs.readFileSync(pidFile, 'utf8').trim());
        expect(Number.isSafeInteger(child) && child > 1).toBe(true);
        expect(result.status, result.stderr).toBe(0);
        expect(result.stdout).toBe('');
        expect(result.stderr.trim().split('\n')).toHaveLength(1);
        await vi.waitFor(
          () => {
            let gone = false;
            try {
              process.kill(child!, 0);
            } catch (error) {
              if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error;
              gone = true;
            }
            expect(gone).toBe(true);
          },
          { timeout: 2000 }
        );
        child = undefined;
        expect(fs.existsSync(sandbox.database)).toBe(false);
      } catch (error) {
        failures.push(error);
      }
      if (child !== undefined && Number.isSafeInteger(child) && child > 1) {
        try {
          process.kill(child, 'SIGKILL');
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== 'ESRCH') failures.push(error);
        }
      }
      if (failures.length) throw new AggregateError(failures, 'Hook cleanup regression');
    });
  });
});
