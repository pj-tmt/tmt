import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import {
  expectError,
  fileSnapshot,
  parseWholeStdout,
  runCli,
  withSandbox,
} from '../support/cli-process.js';

describe('native configuration process boundary', () => {
  it('sets every CLI theme base globally while preserving token overrides and opaque keys', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      const original = {
        theme: { base: 'terminal', waiting: '#e0a458', muted: 'bright black' },
        defaults: { timeout: 30, future: [1, 'keep'] },
        preambleMode: 'disabled',
        future: { arbitrary: [true, null, { nested: 'keep' }] },
      };
      fs.writeFileSync(sandbox.globalConfig, JSON.stringify(original));
      const local = '{"$config":{"preambleEvery":7},"future":"untouched"}';
      fs.writeFileSync(sandbox.localConfig, local);
      for (const base of ['tmt', 'tmt-light', 'terminal', 'mono']) {
        const set = await runCli(sandbox, [
          'config',
          'set',
          '--global',
          'theme.base',
          base,
          '--json',
        ]);
        expect(set.status).toBe(0);
        expect(parseWholeStdout(set)).toEqual({ ok: true });
        expect(JSON.parse(fs.readFileSync(sandbox.globalConfig, 'utf8'))).toEqual({
          ...original,
          theme: { ...original.theme, base },
        });
        const show = await runCli(sandbox, ['config', 'show', '--json']);
        expect(show.status).toBe(0);
        expect(parseWholeStdout(show)).toMatchObject({
          resolved: { theme: { base, waiting: '#e0a458' } },
          sources: { theme: 'global' },
        });
        const human = await runCli(sandbox, ['config', 'show']);
        expect(human.status).toBe(0);
        const row = human.stdout
          .split('\n')
          .find((line) => line.trimStart().startsWith('theme.base'));
        expect(row?.trim().split(/\s{2,}/)).toEqual([
          'theme.base',
          base,
          'global',
          'global CLI',
          'tmt, tmt-light, terminal, mono',
        ]);
        expect(fs.readFileSync(sandbox.localConfig, 'utf8')).toBe(local);
        expect(fs.existsSync(sandbox.database)).toBe(false);
      }
    });
  });

  it('lists valid bases on invalid writes and leaves both configuration files unchanged', async () => {
    await withSandbox(async (sandbox) => {
      for (const base of ['unknown', 'auto', 'TMT', 'tmt ', '']) {
        const before = fileSnapshot(sandbox.root);
        const args = ['config', 'set', '--global', 'theme.base', base];
        const human = await runCli(sandbox, args);
        expect(human.status).toBe(1);
        expect(human.stdout).toBe('');
        expect(human.stderr).toBe(
          `error: Invalid value for theme.base: ${base}\nhint: Valid bases: tmt, tmt-light, terminal, mono\n`
        );
        const json = await runCli(sandbox, [...args, '--json']);
        expect(json.status).toBe(1);
        expect(expectError(json, 'ERROR')).toEqual({
          error: {
            code: 'ERROR',
            message: `Invalid value for theme.base: ${base}.`,
            suggestion: 'Valid bases: tmt, tmt-light, terminal, mono',
          },
        });
        expect(fileSnapshot(sandbox.root)).toEqual(before);
      }
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      fs.writeFileSync(
        sandbox.globalConfig,
        '{"theme":{"base":"mono","waiting":"blue"},"future":1}'
      );
      fs.writeFileSync(
        sandbox.localConfig,
        '{"$config":{"preambleEvery":7},"theme":{"base":"tmt"}}'
      );
      const before = fileSnapshot(sandbox.root);
      for (const args of [
        ['config', 'set', 'theme.base', 'tmt'],
        ['config', 'set', '--global', 'theme.base', 'auto'],
        ['config', 'set', '--global', 'theme.waiting', 'red'],
        ['config', 'rm', 'theme.base'],
      ]) {
        const result = await runCli(sandbox, [...args, '--json']);
        expect(result.status).toBe(1);
        expectError(result, 'ERROR');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
      }
      expect((await runCli(sandbox, ['config', 'rm', '--json'])).status).toBe(0);
      expect(fs.readFileSync(sandbox.globalConfig, 'utf8')).toBe(before['xdg/tmt/config.json']);
      expect(JSON.parse(fs.readFileSync(sandbox.localConfig, 'utf8'))).toEqual({
        theme: { base: 'tmt' },
      });
    });
  });

  it('creates an absent theme and refuses malformed containers without overwriting them', async () => {
    await withSandbox(async (sandbox) => {
      const args = ['config', 'set', '--global', 'theme.base', 'tmt-light', '--json'];
      expect((await runCli(sandbox, args)).status).toBe(0);
      expect(JSON.parse(fs.readFileSync(sandbox.globalConfig, 'utf8'))).toEqual({
        theme: { base: 'tmt-light' },
      });
      expect(parseWholeStdout(await runCli(sandbox, ['config', 'show', '--json']))).toMatchObject({
        resolved: { theme: { base: 'tmt-light' } },
        sources: { theme: 'global' },
        themeError: null,
      });
      for (const theme of [null, 'mono', 3, ['tmt']]) {
        fs.writeFileSync(sandbox.globalConfig, JSON.stringify({ theme, future: 'keep' }));
        const before = fileSnapshot(sandbox.root);
        const result = await runCli(sandbox, args);
        expect(result.status).toBe(1);
        expectError(result, 'CONFIG_ERROR');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
        expect((await runCli(sandbox, ['config', 'show', '--json'])).status).toBe(0);
      }
    });
  });

  it('shows the pane badge on by default and keeps an explicit off', async () => {
    await withSandbox(async (sandbox) => {
      const badge = async () => {
        const shown = parseWholeStdout(await runCli(sandbox, ['config', '--json']));
        return [
          (shown.resolved as { ui: { paneBadge: string } }).ui.paneBadge,
          (shown.sources as { ui: { paneBadge: string } }).ui.paneBadge,
        ];
      };
      expect(await badge()).toEqual(['on', 'default']);
      expect(
        (await runCli(sandbox, ['config', 'set', 'ui.paneBadge', 'off', '--global', '--json']))
          .status
      ).toBe(0);
      expect(await badge()).toEqual(['off', 'global']);
    });
  });

  it('renders resolved configuration values with their sources in human mode', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      fs.writeFileSync(
        sandbox.globalConfig,
        JSON.stringify({
          preambleMode: 'disabled',
          defaults: { preambleEvery: 7, timeout: 30 },
          exchange: { retentionDays: 365 },
          ui: { paneBadge: 'on' },
        })
      );
      fs.writeFileSync(sandbox.localConfig, JSON.stringify({ $config: { preambleEvery: 0 } }));
      const before = fileSnapshot(sandbox.root);

      const result = await runCli(sandbox, ['config', 'show']);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      expect(result.stdout.startsWith('SETTINGS\n')).toBe(true);
      const rows = result.stdout
        .split('\n')
        .map((line) => line.trim())
        .filter(
          (line) =>
            line.startsWith('preamble') ||
            line.startsWith('pasteEnter') ||
            line.startsWith('defaults.') ||
            line.startsWith('exchange.') ||
            line.startsWith('ui.') ||
            line.startsWith('notifications.') ||
            line.startsWith('experimental.') ||
            line.startsWith('notes.')
        )
        .map((line) => line.split(/\s{2,}/));
      expect(rows).toEqual([
        ['experimental.channel', 'false', 'default', 'global CLI', 'true or false'],
        ['notes.compactionReminder', 'true', 'default', 'global CLI', 'true or false'],
        [
          'notifications.replyBatchWindowMs',
          '5000',
          'default',
          'global CLI',
          'an integer from 0 through 60000 milliseconds',
        ],
        [
          'notifications.typingQuietMs',
          '2000',
          'default',
          'global CLI',
          'an integer from 0 through 30000 milliseconds',
        ],
        ['preambleMode', 'disabled', 'global', 'local/global CLI', "'always' or 'disabled'"],
        ['preambleEvery', '0', 'local', 'local/global CLI', 'a safe non-negative integer'],
        [
          'pasteEnterDelayMs',
          '500',
          'default',
          'local/global CLI',
          'a finite number from 0 through 2147483647',
        ],
        [
          'defaults.timeout',
          '30',
          'global',
          'global file only',
          'a finite positive number no greater than 86400',
        ],
        ['defaults.pollInterval', '1', 'default', 'global file only', 'a finite positive number'],
        [
          'defaults.captureLines',
          '100',
          'default',
          'global file only',
          'an integer from 0 through 2147483647',
        ],
        ['exchange.retentionDays', '365', 'global', 'global CLI', 'an integer from 1 through 3650'],
        ['ui.paneBadge', 'on', 'global', 'global CLI', "'on' or 'off'"],
      ]);
      expect(result.stdout).toContain(
        'hint: CLI numeric writes use unsigned decimal integers; config rm removes local overrides only\n'
      );
      expect(result.stdout).toContain('\nPATHS\n');
      expect(result.stdout).toContain(`  global  ${sandbox.globalConfig}\n`);
      expect(result.stdout).toContain(`  local   ${fs.realpathSync(sandbox.localConfig)}\n`);
      expect(fileSnapshot(sandbox.root)).toEqual(before);
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  });

  it('edits default-on workspace snapshot policy globally and preserves opaque siblings', async () => {
    await withSandbox(async (sandbox) => {
      const initial = parseWholeStdout(await runCli(sandbox, ['config', 'show', '--json']));
      expect(initial).toMatchObject({
        resolved: { workspace: { snapshotEnabled: true, snapshotIntervalMs: 60000 } },
        sources: { workspace: { snapshotEnabled: 'default', snapshotIntervalMs: 'default' } },
      });
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      fs.writeFileSync(
        sandbox.globalConfig,
        JSON.stringify({ workspace: { future: ['keep'] }, other: 7 })
      );
      for (const [key, value] of [
        ['snapshotEnabled', 'false'],
        ['snapshotIntervalMs', '0'],
      ]) {
        const result = await runCli(sandbox, [
          'config',
          'set',
          '--global',
          `workspace.${key}`,
          value,
          '--json',
        ]);
        expect(result.status).toBe(0);
        expect(parseWholeStdout(result)).toEqual({ ok: true });
      }
      expect(JSON.parse(fs.readFileSync(sandbox.globalConfig, 'utf8'))).toEqual({
        workspace: { future: ['keep'], snapshotEnabled: false, snapshotIntervalMs: 0 },
        other: 7,
      });
      const before = fileSnapshot(sandbox.root);
      for (const args of [
        ['config', 'set', 'workspace.snapshotEnabled', 'true'],
        ['config', 'rm', 'workspace.snapshotEnabled'],
        ['config', 'set', '--global', 'workspace.snapshotEnabled', '1'],
        ['config', 'set', '--global', 'workspace.snapshotIntervalMs', '2147483648'],
      ]) {
        const result = await runCli(sandbox, [...args, '--json']);
        expect(result.status).toBe(1);
        expectError(result, 'ERROR');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
      }
      expect(fs.existsSync(sandbox.database)).toBe(false);
      expect(fs.existsSync(path.join(sandbox.globalDir, 'workspace'))).toBe(false);
    });
  });

  it('keeps experimental channels off by default and validates global-only opt-in without launch effects', async () => {
    await withSandbox(async (sandbox) => {
      expect(parseWholeStdout(await runCli(sandbox, ['config', 'show', '--json']))).toMatchObject({
        resolved: { experimental: { channel: false } },
        sources: { experimental: { channel: 'default' } },
      });
      for (const driver of ['claude', 'codex']) {
        for (const args of [
          ['run', '--channel', 'Agent', driver],
          ['run', '--resume', '--channel', 'Agent'],
          ['resume', '--channel', 'Agent'],
        ]) {
          const before = fileSnapshot(sandbox.root);
          const result = await runCli(sandbox, args);
          expect(result.status).toBe(1);
          expect(result.stderr.trim()).toBe(
            'error: Message channels require experimental.channel=true.'
          );
          expect(fileSnapshot(sandbox.root)).toEqual(before);
        }
      }
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      fs.writeFileSync(sandbox.globalConfig, JSON.stringify({ experimental: { future: 'keep' } }));
      for (const value of ['true', 'false']) {
        expect(
          (await runCli(sandbox, ['config', 'set', '--global', 'experimental.channel', value]))
            .status
        ).toBe(0);
        expect(JSON.parse(fs.readFileSync(sandbox.globalConfig, 'utf8'))).toEqual({
          experimental: { future: 'keep', channel: value === 'true' },
        });
      }
      const before = fileSnapshot(sandbox.root);
      for (const args of [
        ['config', 'set', 'experimental.channel', 'true'],
        ['config', 'rm', 'experimental.channel'],
        ['config', 'set', '--global', 'experimental.channel', '1'],
      ]) {
        expectError(await runCli(sandbox, [...args, '--json']), 'ERROR');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
      }
      for (const value of ['true', 1, null, {}]) {
        fs.writeFileSync(
          sandbox.globalConfig,
          JSON.stringify({ experimental: { channel: value } })
        );
        expectError(await runCli(sandbox, ['config', 'show', '--json']), 'CONFIG_ERROR');
      }
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  });

  it('edits the global notes reminder boolean without creating notes or storage', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      fs.writeFileSync(sandbox.globalConfig, JSON.stringify({ notes: { future: 'keep' } }));
      expect(
        (
          await runCli(sandbox, [
            'config',
            'set',
            'notes.compactionReminder',
            'false',
            '--global',
            '--json',
          ])
        ).status
      ).toBe(0);
      expect(JSON.parse(fs.readFileSync(sandbox.globalConfig, 'utf8'))).toEqual({
        notes: { future: 'keep', compactionReminder: false },
      });
      expect(parseWholeStdout(await runCli(sandbox, ['config', 'show', '--json']))).toMatchObject({
        resolved: { notes: { compactionReminder: false } },
        sources: { notes: { compactionReminder: 'global' } },
      });
      const before = fileSnapshot(sandbox.root);
      for (const args of [
        ['config', 'set', 'notes.compactionReminder', 'true'],
        ['config', 'rm', 'notes.compactionReminder'],
        ['config', 'set', 'notes.compactionReminder', '1', '--global'],
        ['config', 'set', 'notes.compactionReminder', 'TRUE', '--global'],
      ]) {
        expectError(await runCli(sandbox, [...args, '--json']), 'ERROR');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
      }
      for (const value of ['false', 0, null, {}]) {
        fs.writeFileSync(
          sandbox.globalConfig,
          JSON.stringify({ notes: { compactionReminder: value } })
        );
        expectError(await runCli(sandbox, ['config', 'show', '--json']), 'CONFIG_ERROR');
      }
      expect(fs.existsSync(path.join(sandbox.globalDir, 'notes'))).toBe(false);
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  });

  it('edits global notification windows, preserves siblings and rejects local writes', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      fs.writeFileSync(sandbox.globalConfig, JSON.stringify({ notifications: { future: 'keep' } }));
      for (const [key, value] of [
        ['replyBatchWindowMs', '0'],
        ['typingQuietMs', '1200'],
      ]) {
        expect(
          (
            await runCli(sandbox, [
              'config',
              'set',
              `notifications.${key}`,
              value,
              '--global',
              '--json',
            ])
          ).status
        ).toBe(0);
      }
      expect(JSON.parse(fs.readFileSync(sandbox.globalConfig, 'utf8'))).toEqual({
        notifications: { future: 'keep', replyBatchWindowMs: 0, typingQuietMs: 1200 },
      });
      const before = fileSnapshot(sandbox.root);
      for (const args of [
        ['config', 'set', 'notifications.replyBatchWindowMs', '1'],
        ['config', 'clear', 'notifications.typingQuietMs'],
        ['config', 'set', 'notifications.replyBatchWindowMs', '60001', '--global'],
        ['config', 'set', 'notifications.typingQuietMs', '2s', '--global'],
      ]) {
        expectError(await runCli(sandbox, [...args, '--json']), 'ERROR');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
      }
      expect(parseWholeStdout(await runCli(sandbox, ['config', 'show', '--json']))).toMatchObject({
        resolved: { notifications: { replyBatchWindowMs: 0, typingQuietMs: 1200 } },
        sources: { notifications: { replyBatchWindowMs: 'global', typingQuietMs: 'global' } },
      });
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  });

  it('retains JavaScript numeric semantics and insertion order in opaque fields', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      const bytes =
        '{"zFuture":9007199254740993,"aFuture":{"values":[1e400,-1e400,-0,3]},"preambleMode":"always"}';
      fs.writeFileSync(sandbox.globalConfig, bytes);
      const shown = await runCli(sandbox, ['config', '--json']);
      expect(shown.status).toBe(0);
      expect(parseWholeStdout(shown)).toMatchObject({ resolved: { preambleMode: 'always' } });
      expect(fs.readFileSync(sandbox.globalConfig, 'utf8')).toBe(bytes);
      const edited = await runCli(sandbox, [
        'config',
        'set',
        'preambleMode',
        'disabled',
        '--global',
        '--json',
      ]);
      expect(edited.status).toBe(0);
      const expected = JSON.parse(bytes);
      expected.preambleMode = 'disabled';
      expect(fs.readFileSync(sandbox.globalConfig, 'utf8')).toBe(
        `${JSON.stringify(expected, null, 2)}\n`
      );
      fs.writeFileSync(sandbox.globalConfig, '{"defaults":{"timeout":1e400}}');
      const invalid = await runCli(sandbox, ['config', '--json']);
      expect(invalid.status).toBe(1);
      expect(expectError(invalid, 'CONFIG_ERROR').error).toMatchObject({
        message: expect.stringContaining('(defaults.timeout)'),
      });
    });
  });

  it('projects exact built-in defaults when config sections are omitted', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      const globalBytes = JSON.stringify({ futureGlobal: { keep: true } });
      const localBytes = JSON.stringify({ keep: true });
      fs.writeFileSync(sandbox.globalConfig, globalBytes);
      fs.writeFileSync(sandbox.localConfig, localBytes);
      const before = fileSnapshot(sandbox.root);

      const result = await runCli(sandbox, ['config', 'show', '--json']);
      expect(result.status).toBe(0);
      const document = parseWholeStdout(result) as {
        resolved: Record<string, unknown>;
        sources: Record<string, unknown>;
      };
      expect(document.resolved).toEqual({
        preambleMode: 'always',
        preambleEvery: 3,
        pasteEnterDelayMs: 500,
        defaults: {
          timeout: 180,
          pollInterval: 1,
          captureLines: 100,
          preambleEvery: 3,
          pasteEnterDelayMs: 500,
        },
        exchange: { retentionDays: 90 },
        ui: { paneBadge: 'on' },
        notifications: { replyBatchWindowMs: 5000, typingQuietMs: 2000 },
        experimental: { channel: false },
        notes: { compactionReminder: true },
        workspace: { snapshotEnabled: true, snapshotIntervalMs: 60000 },
        theme: {},
      });
      expect(document.sources).toEqual({
        preambleMode: 'default',
        preambleEvery: 'default',
        pasteEnterDelayMs: 'default',
        exchange: { retentionDays: 'default' },
        ui: { paneBadge: 'default' },
        notifications: { replyBatchWindowMs: 'default', typingQuietMs: 'default' },
        experimental: { channel: 'default' },
        notes: { compactionReminder: 'default' },
        workspace: { snapshotEnabled: 'default', snapshotIntervalMs: 'default' },
        theme: 'default',
      });
      expect(fileSnapshot(sandbox.root)).toEqual(before);
      expect(fs.readFileSync(sandbox.globalConfig, 'utf8')).toBe(globalBytes);
      expect(fs.readFileSync(sandbox.localConfig, 'utf8')).toBe(localBytes);
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  });

  it('reports the theme as written, names a bad key without failing, and never lets it break a command', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      fs.writeFileSync(
        sandbox.globalConfig,
        JSON.stringify({ theme: { base: 'tmt-light', waiting: '#e0a458' } })
      );
      const shown = await runCli(sandbox, ['config', 'show', '--json']);
      expect(shown.status).toBe(0);
      expect(parseWholeStdout(shown)).toMatchObject({
        resolved: { theme: { base: 'tmt-light', waiting: '#e0a458' } },
        sources: { theme: 'global' },
        themeError: null,
      });
      const text = await runCli(sandbox, ['config', 'show']);
      expect(text.status).toBe(0);
      expect(text.stdout).toMatch(/theme\.base\s+tmt-light/);
      expect(text.stdout).toMatch(/theme\.waiting\s+#e0a458/);

      for (const [theme, key] of [
        [{ waiting: 'orange' }, 'theme.waiting'],
        [{ base: 'dark' }, 'theme.base'],
        [{ error: 'red' }, 'theme.error'],
        [{ waiting: 3 }, 'theme.waiting'],
        [['tmt'], 'theme'],
      ] as const) {
        fs.writeFileSync(sandbox.globalConfig, JSON.stringify({ theme }));
        // An extension finds its own file through config show: a bad theme is
        // reported by its key and never fails the command.
        const invalid = await runCli(sandbox, ['config', 'show', '--json']);
        expect(invalid.status).toBe(0);
        expect(parseWholeStdout(invalid)).toMatchObject({
          themeError: { key, message: expect.any(String) },
          paths: { global: sandbox.globalConfig },
        });
        const human = await runCli(sandbox, ['config', 'show']);
        expect(human.status).toBe(0);
        expect(human.stderr).toContain(key);
        // Every other command keeps working with the terminal's colors.
        const listed = await runCli(sandbox, ['ls', '--json']);
        expect(listed.status).toBe(0);
      }
    });
  });

  it('accepts a safe preamble frequency above the timer delay bound', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      const globalBytes = JSON.stringify({ defaults: { preambleEvery: 2_147_483_648 } });
      fs.writeFileSync(sandbox.globalConfig, globalBytes);

      const result = await runCli(sandbox, ['config', 'show', '--json']);
      expect(result.status).toBe(0);
      const document = parseWholeStdout(result) as {
        resolved: { preambleEvery: number; defaults: { preambleEvery: number } };
        sources: { preambleEvery: string };
      };
      expect(document.resolved).toMatchObject({
        preambleEvery: 2_147_483_648,
        defaults: { preambleEvery: 2_147_483_648 },
      });
      expect(document.sources).toMatchObject({ preambleEvery: 'global' });
      expect(fs.readFileSync(sandbox.globalConfig, 'utf8')).toBe(globalBytes);
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  });

  it('projects zero values with local precedence while omitting opaque keys', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      const globalValue = {
        preambleMode: 'disabled',
        mode: 'retired-global-mode',
        futureGlobal: { keep: true },
        defaults: {
          timeout: 86_400,
          pollInterval: 0.25,
          captureLines: 0,
          preambleEvery: 7,
          pasteEnterDelayMs: 1.5,
          maxCaptureLines: 999,
          futureDefault: 'opaque',
        },
      };
      const localValue = {
        keep: { value: true },
        $config: {
          mode: 'retired-local-mode',
          preambleMode: 'always',
          preambleEvery: 0,
          futureLocal: ['opaque'],
        },
      };
      const globalBytes = JSON.stringify(globalValue);
      const localBytes = JSON.stringify(localValue);
      fs.writeFileSync(sandbox.globalConfig, globalBytes);
      fs.writeFileSync(sandbox.localConfig, localBytes);
      const before = fileSnapshot(sandbox.root);

      const result = await runCli(sandbox, ['config', 'show', '--json']);
      expect(result.status).toBe(0);
      const document = parseWholeStdout(result) as {
        resolved: Record<string, unknown>;
        sources: Record<string, unknown>;
      };
      expect(document.resolved).toEqual({
        preambleMode: 'always',
        preambleEvery: 0,
        pasteEnterDelayMs: 1.5,
        defaults: {
          timeout: 86_400,
          pollInterval: 0.25,
          captureLines: 0,
          preambleEvery: 0,
          pasteEnterDelayMs: 1.5,
        },
        exchange: { retentionDays: 90 },
        ui: { paneBadge: 'on' },
        notifications: { replyBatchWindowMs: 5000, typingQuietMs: 2000 },
        experimental: { channel: false },
        notes: { compactionReminder: true },
        workspace: { snapshotEnabled: true, snapshotIntervalMs: 60000 },
        theme: {},
      });
      expect(document.sources).toEqual({
        preambleMode: 'local',
        preambleEvery: 'local',
        pasteEnterDelayMs: 'global',
        exchange: { retentionDays: 'default' },
        ui: { paneBadge: 'default' },
        notifications: { replyBatchWindowMs: 'default', typingQuietMs: 'default' },
        experimental: { channel: 'default' },
        notes: { compactionReminder: 'default' },
        workspace: { snapshotEnabled: 'default', snapshotIntervalMs: 'default' },
        theme: 'default',
      });
      expect(fileSnapshot(sandbox.root)).toEqual(before);
      expect(fs.readFileSync(sandbox.globalConfig, 'utf8')).toBe(globalBytes);
      expect(fs.readFileSync(sandbox.localConfig, 'utf8')).toBe(localBytes);
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  });

  it('normalizes XDG parent components without requiring discarded directories', async () => {
    await withSandbox(async (sandbox) => {
      sandbox.env.XDG_CONFIG_HOME = path.join(sandbox.root, 'missing') + '/../resolved';
      const shown = await runCli(sandbox, ['config', '--json']);
      expect(shown.status).toBe(0);
      const config = path.join(sandbox.root, 'resolved', 'tmt', 'config.json');
      expect(parseWholeStdout(shown)).toMatchObject({ paths: { global: config } });
      expect(
        (await runCli(sandbox, ['config', 'set', 'ui.paneBadge', 'on', '--global', '--json']))
          .status
      ).toBe(0);
      expect(JSON.parse(fs.readFileSync(config, 'utf8'))).toEqual({ ui: { paneBadge: 'on' } });
      expect(fs.existsSync(path.join(sandbox.root, 'missing'))).toBe(false);
    });
  });

  it('normalizes HOME before refusing an unmanaged former-default move', async () => {
    await withSandbox(async (sandbox) => {
      sandbox.env.HOME = path.join(sandbox.root, 'missing') + '/../resolved-home';
      delete sandbox.env.XDG_CONFIG_HOME;
      const former = path.join(sandbox.root, 'resolved-home', '.tmux-team', 'config.json');
      const config = path.join(sandbox.root, 'resolved-home', '.config', 'tmt', 'config.json');
      fs.mkdirSync(path.dirname(former), { recursive: true });
      fs.writeFileSync(former, '{"ui":{"paneBadge":"on"},"opaque":"retained"}');
      const before = fileSnapshot(sandbox.root);
      const shown = await runCli(sandbox, ['config', '--json']);
      expect(shown.status).not.toBe(0);
      const diagnostic = shown.stdout + shown.stderr;
      expect(diagnostic).toContain(path.dirname(former));
      expect(diagnostic).toContain(path.dirname(config));
      expect(diagnostic).toContain('TMT_HOME');
      expect(diagnostic).not.toContain('missing/..');
      expect(fileSnapshot(sandbox.root)).toEqual(before);
      expect(fs.existsSync(path.dirname(config))).toBe(false);
      expect(fs.existsSync(path.join(sandbox.root, 'missing'))).toBe(false);
    });
  });

  it('preserves a file blocking the configuration directory on write failure', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.xdgConfigHome, { recursive: true });
      fs.writeFileSync(sandbox.globalDir, 'unrelated user file');
      const before = fileSnapshot(sandbox.root);
      const result = await runCli(sandbox, [
        'config',
        'set',
        'ui.paneBadge',
        'on',
        '--global',
        '--json',
      ]);
      expect(result.status).toBe(1);
      expectError(result, 'INTERNAL_ERROR');
      expect(fileSnapshot(sandbox.root)).toEqual(before);
    });
  });

  it('validates known settings and containers before projecting overrides', async () => {
    for (const [global, local] of [
      [null, {}],
      [[], {}],
      [{ defaults: null }, {}],
      [{ exchange: [] }, {}],
      [{ ui: { paneBadge: true } }, {}],
      [{ defaults: { timeout: '180' } }, {}],
      [{ defaults: { pollInterval: 0 } }, {}],
      [{ defaults: { captureLines: 0.5 } }, {}],
      [{ exchange: { retentionDays: 3651 } }, {}],
      [{ defaults: { preambleEvery: null } }, { $config: { preambleEvery: 3 } }],
      [{}, { $config: [] }],
      [{}, { $config: { pasteEnterDelayMs: null } }],
    ]) {
      await withSandbox(async (sandbox) => {
        fs.mkdirSync(sandbox.globalDir, { recursive: true });
        fs.writeFileSync(sandbox.globalConfig, JSON.stringify(global));
        fs.writeFileSync(sandbox.localConfig, JSON.stringify(local));
        const before = fileSnapshot(sandbox.root);
        const result = await runCli(sandbox, ['config', 'show', '--json']);
        expect(result.status).toBe(1);
        expectError(result, 'CONFIG_ERROR');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
        expect(fs.existsSync(sandbox.database)).toBe(false);
      });
    }
  });

  it('reports malformed JSON without replacing or repairing it', async () => {
    await withSandbox(async (sandbox) => {
      fs.writeFileSync(sandbox.localConfig, '{ not json');
      const before = fileSnapshot(sandbox.root);
      const result = await runCli(sandbox, ['config', 'set', 'preambleEvery', '4', '--json']);
      expect(result.status).toBe(1);
      expect(expectError(result, 'CONFIG_ERROR').error).toMatchObject({
        message: expect.stringContaining(sandbox.localConfig),
      });
      expect(fileSnapshot(sandbox.root)).toEqual(before);
    });
  });

  it('keeps global-only settings out of local edits and ignores opaque local copies', async () => {
    await withSandbox(async (sandbox) => {
      fs.writeFileSync(
        sandbox.localConfig,
        JSON.stringify({
          $config: { exchange: { retentionDays: 1 }, ui: { paneBadge: 'on' }, future: true },
        })
      );
      const before = fileSnapshot(sandbox.root);
      for (const args of [
        ['config', 'set', 'exchange.retentionDays', '1'],
        ['config', 'set', 'ui.paneBadge', 'on'],
        ['config', 'clear', 'exchange.retentionDays'],
        ['config', 'clear', 'ui.paneBadge'],
      ]) {
        const result = await runCli(sandbox, [...args, '--json']);
        expect(result.status).toBe(1);
        expectError(result, 'ERROR');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
      }
      const shown = await runCli(sandbox, ['config', '--json']);
      expect(shown.status).toBe(0);
      expect(parseWholeStdout(shown)).toMatchObject({
        resolved: { exchange: { retentionDays: 90 }, ui: { paneBadge: 'on' } },
      });
      expect(fileSnapshot(sandbox.root)).toEqual(before);
    });
  });

  it('rejects invalid global and local setter values without rewriting files', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      fs.writeFileSync(
        sandbox.globalConfig,
        JSON.stringify({ defaults: { preambleEvery: 3, pasteEnterDelayMs: 500 } })
      );
      fs.writeFileSync(
        sandbox.localConfig,
        JSON.stringify({ $config: { preambleEvery: 3, pasteEnterDelayMs: 500 } })
      );
      const before = fileSnapshot(sandbox.root);
      const invalidCases: Array<{
        key: string;
        value: string;
        global?: boolean;
      }> = [
        { key: 'preambleEvery', value: '1.5' },
        { key: 'preambleEvery', value: '12junk' },
        { key: 'preambleEvery', value: ' 12' },
        { key: 'preambleEvery', value: '+12' },
        { key: 'preambleEvery', value: '9007199254740992' },
        { key: 'preambleEvery', value: '-1' },
        { key: 'pasteEnterDelayMs', value: '1.5' },
        { key: 'pasteEnterDelayMs', value: '12junk' },
        { key: 'pasteEnterDelayMs', value: '12\n' },
        { key: 'pasteEnterDelayMs', value: ' 12' },
        { key: 'pasteEnterDelayMs', value: '+12' },
        { key: 'pasteEnterDelayMs', value: '2147483648' },
        { key: 'pasteEnterDelayMs', value: '-1' },
        { key: 'preambleEvery', value: '1.5', global: true },
        { key: 'preambleEvery', value: '9007199254740992', global: true },
        { key: 'pasteEnterDelayMs', value: '2147483648', global: true },
        { key: 'exchange.retentionDays', value: '0', global: true },
        { key: 'exchange.retentionDays', value: '3651', global: true },
        { key: 'defaults.timeout', value: '60', global: true },
        { key: 'defaults.pollInterval', value: '2', global: true },
        { key: 'defaults.captureLines', value: '20', global: true },
        { key: 'futureKey', value: '1' },
        { key: 'futureKey', value: '1', global: true },
      ];

      for (const invalid of invalidCases) {
        const args = ['config', 'set', invalid.key, invalid.value];
        if (invalid.global) args.push('--global');
        args.push('--json');
        const result = await runCli(sandbox, args);
        expect(result.status, `${invalid.key}=${invalid.value}`).toBe(1);
        expectError(result, 'ERROR');
        expect(fileSnapshot(sandbox.root), `${invalid.key}=${invalid.value}`).toEqual(before);
      }
      for (const key of ['defaults.timeout', 'defaults.pollInterval', 'defaults.captureLines']) {
        const cleared = await runCli(sandbox, ['config', 'clear', key, '--json']);
        expect(cleared.status).toBe(1);
        expectError(cleared, 'ERROR');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
      }
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  }, 30_000);

  it('repairs only targeted settings while preserving opaque siblings and zero values', async () => {
    await withSandbox(async (sandbox) => {
      fs.mkdirSync(sandbox.globalDir, { recursive: true });
      fs.writeFileSync(
        sandbox.globalConfig,
        JSON.stringify({ defaults: { timeout: 240, futureDefault: { keep: true } } })
      );
      fs.writeFileSync(
        sandbox.localConfig,
        JSON.stringify({
          keep: 'opaque',
          $config: { preambleEvery: 'invalid', pasteEnterDelayMs: 500 },
        })
      );

      const repaired = await runCli(sandbox, ['config', 'set', 'preambleEvery', '4', '--json']);
      expect(repaired.status).toBe(0);
      expect(parseWholeStdout(repaired)).toEqual({ ok: true });
      expect(JSON.parse(fs.readFileSync(sandbox.localConfig, 'utf8'))).toEqual({
        keep: 'opaque',
        $config: { preambleEvery: 4, pasteEnterDelayMs: 500 },
      });

      const globalSet = await runCli(sandbox, [
        'config',
        'set',
        'preambleEvery',
        '0',
        '--global',
        '--json',
      ]);
      expect(globalSet.status).toBe(0);
      expect(parseWholeStdout(globalSet)).toEqual({ ok: true });
      expect(JSON.parse(fs.readFileSync(sandbox.globalConfig, 'utf8'))).toEqual({
        defaults: {
          timeout: 240,
          preambleEvery: 0,
          futureDefault: { keep: true },
        },
      });

      const zeroPaste = await runCli(sandbox, [
        'config',
        'set',
        'pasteEnterDelayMs',
        '0',
        '--json',
      ]);
      expect(zeroPaste.status).toBe(0);
      expect(parseWholeStdout(zeroPaste)).toEqual({ ok: true });
      expect(JSON.parse(fs.readFileSync(sandbox.localConfig, 'utf8'))).toEqual({
        keep: 'opaque',
        $config: { preambleEvery: 4, pasteEnterDelayMs: 0 },
      });
      fs.writeFileSync(
        sandbox.localConfig,
        JSON.stringify({
          keep: 'opaque',
          $config: { preambleEvery: 'invalid', pasteEnterDelayMs: 0 },
        })
      );
      const cleared = await runCli(sandbox, ['config', 'clear', 'preambleEvery', '--json']);
      expect(cleared.status).toBe(0);
      expect(parseWholeStdout(cleared)).toEqual({ ok: true });
      expect(JSON.parse(fs.readFileSync(sandbox.localConfig, 'utf8'))).toEqual({
        keep: 'opaque',
        $config: { pasteEnterDelayMs: 0 },
      });

      const shown = await runCli(sandbox, ['config', 'show', '--json']);
      expect(shown.status).toBe(0);
      const document = parseWholeStdout(shown) as {
        resolved: Record<string, unknown>;
        sources: Record<string, unknown>;
      };
      expect(document.resolved).toMatchObject({
        preambleEvery: 0,
        pasteEnterDelayMs: 0,
        defaults: { timeout: 240, preambleEvery: 0, pasteEnterDelayMs: 0 },
      });
      expect(document.sources).toMatchObject({
        preambleEvery: 'global',
        pasteEnterDelayMs: 'local',
      });

      const invalidRemainder = JSON.stringify({
        keep: 'opaque',
        $config: { preambleEvery: 'invalid', pasteEnterDelayMs: 'invalid' },
      });
      fs.writeFileSync(sandbox.localConfig, invalidRemainder);
      const rejected = await runCli(sandbox, ['config', 'clear', 'preambleEvery', '--json']);
      expect(rejected.status).toBe(1);
      expectError(rejected, 'CONFIG_ERROR');
      expect(fs.readFileSync(sandbox.localConfig, 'utf8')).toBe(invalidRemainder);
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  }, 30_000);

  it('distinguishes absent-container clear, clear-all and obsolete-key repair', async () => {
    await withSandbox(async (sandbox) => {
      const before = fileSnapshot(sandbox.root);
      const missing = await runCli(sandbox, ['config', 'clear', 'preambleEvery', '--json']);
      expect(missing.status).toBe(0);
      expect(parseWholeStdout(missing)).toEqual({ ok: true });
      expect(fileSnapshot(sandbox.root)).toEqual(before);
      expect((await runCli(sandbox, ['config', 'clear', '--json'])).status).toBe(0);
      expect(JSON.parse(fs.readFileSync(sandbox.localConfig, 'utf8'))).toEqual({});
      fs.writeFileSync(
        sandbox.localConfig,
        JSON.stringify({
          keep: 'data',
          $config: { mode: 'legacy' },
        })
      );
      expect((await runCli(sandbox, ['config', 'clear', 'mode', '--json'])).status).toBe(0);
      expect(JSON.parse(fs.readFileSync(sandbox.localConfig, 'utf8'))).toEqual({ keep: 'data' });
    });
  });

  it('finds the nearest ancestor settings file and edits that file only', async () => {
    await withSandbox(async (sandbox) => {
      const nested = path.join(sandbox.cwd, 'child', 'nested');
      fs.mkdirSync(nested, { recursive: true });
      fs.writeFileSync(sandbox.localConfig, JSON.stringify({ $config: { preambleEvery: 8 } }));
      const child = { ...sandbox, cwd: nested };
      const shown = await runCli(child, ['config', '--json']);
      expect(shown.status).toBe(0);
      expect(parseWholeStdout(shown)).toMatchObject({
        resolved: { preambleEvery: 8 },
        sources: { preambleEvery: 'local' },
        paths: { local: fs.realpathSync(sandbox.localConfig) },
      });
      expect((await runCli(child, ['config', 'set', 'preambleEvery', '9', '--json'])).status).toBe(
        0
      );
      expect(JSON.parse(fs.readFileSync(sandbox.localConfig, 'utf8'))).toEqual({
        $config: { preambleEvery: 9 },
      });
      expect(fs.readdirSync(nested)).toEqual([]);
      expect(fs.existsSync(sandbox.database)).toBe(false);
    });
  });

  it.each(['new', 'legacy-empty', 'legacy-config', 'both-config', 'xdg-empty'])(
    'refuses unmanaged cutover of the %s former default without creating state',
    async (scenario) => {
      await withSandbox(async (sandbox) => {
        delete sandbox.env.XDG_CONFIG_HOME;
        const xdg = path.join(sandbox.home, '.config', 'tmux-team');
        const legacy = path.join(sandbox.home, '.tmux-team');
        const current = path.join(sandbox.home, '.config', 'tmt');
        if (scenario !== 'new') fs.mkdirSync(legacy, { recursive: true });
        if (
          scenario === 'legacy-config' ||
          scenario === 'both-config' ||
          scenario === 'xdg-empty'
        ) {
          fs.writeFileSync(path.join(legacy, 'config.json'), '{}');
        }
        if (scenario === 'both-config' || scenario === 'xdg-empty')
          fs.mkdirSync(xdg, { recursive: true });
        if (scenario === 'both-config') fs.writeFileSync(path.join(xdg, 'config.json'), '{}');
        const before = fileSnapshot(sandbox.root);
        const shown = await runCli(sandbox, ['config', '--json']);
        if (scenario === 'new') {
          expect(shown.status).toBe(0);
          expect(shown.stderr).toBe('');
          expect(parseWholeStdout(shown)).toMatchObject({
            paths: { global: path.join(current, 'config.json') },
          });
        } else {
          expect(shown.status).not.toBe(0);
          const diagnostic = shown.stdout + shown.stderr;
          expect(diagnostic).toContain(scenario === 'both-config' ? xdg : legacy);
          expect(diagnostic).toContain(current);
          expect(diagnostic).toContain('TMT_HOME');
          expect(diagnostic).toContain('unmanaged executable');
        }
        expect(fileSnapshot(sandbox.root)).toEqual(before);
        expect(fs.existsSync(current)).toBe(false);
        if (scenario === 'legacy-empty' || scenario === 'legacy-config')
          expect(fs.existsSync(path.dirname(current))).toBe(false);
      });
    }
  );

  it('honors the explicit directory override within the isolated process group', async () => {
    await withSandbox(async (sandbox) => {
      const override = path.join(sandbox.root, "custom root's files");
      // The common launcher intentionally removes ambient TMT_HOME.
      // env introduces this explicit fixture-only value inside its bounded child
      // group; no shell expansion or production environment bypass is added.
      const selected = {
        ...sandbox,
        cli: {
          executable: '/usr/bin/env',
          args: [`TMT_HOME=${override}`, sandbox.cli.executable, ...sandbox.cli.args],
        },
      };
      const result = await runCli(selected, [
        'config',
        'set',
        'ui.paneBadge',
        'on',
        '--global',
        '--json',
      ]);
      expect(result.status).toBe(0);
      expect(parseWholeStdout(result)).toEqual({ ok: true });
      expect(JSON.parse(fs.readFileSync(path.join(override, 'config.json'), 'utf8'))).toEqual({
        ui: { paneBadge: 'on' },
      });
      expect(fs.existsSync(sandbox.globalDir)).toBe(false);
      expect(fs.existsSync(path.join(override, 'tmux-team.db'))).toBe(false);
    });
  });
});
