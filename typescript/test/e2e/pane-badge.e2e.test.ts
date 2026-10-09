import { writeExecutable } from '../support/executable-fixture.mjs';
import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vite-plus/test';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { durableIdentity, durableState } from './identity-state-oracle.js';
import { waitForFileContent } from './wait-for-file.js';

const BADGE_OPTION = '@tmt.badge';
const BORDER_OWNER = '@tmt.border';

function localOption(fixture: E2EFixture, option: string, pane = fixture.pane): string {
  return fixture.tmux(['show-options', '-p', '-qv', '-t', pane, option]);
}
const USER_FORMAT = '#[align=left]#{window_index}.#{pane_index}#[align=right]repo/branch';
const BADGE_FRAGMENT = '#{?@tmt.badge, [#{@tmt.badge}],}';
const NARROW_BADGE_FRAGMENT = '#{?#{&&:#{@tmt.badge},#{e|>=:#{pane_width},80}}, [#{@tmt.badge}],}';

function badge(fixture: E2EFixture): string {
  // The minimal Docker image has no UTF-8 locale. Request UTF-8 output so tmux
  // does not render wide characters as underscores in this read-side oracle.
  return fixture.tmux(['-u', 'show-options', '-p', '-qv', '-t', fixture.pane, BADGE_OPTION]).trim();
}

function appearance(fixture: E2EFixture) {
  return {
    globalFormat: fixture.tmux(['show-options', '-gw', '-v', 'pane-border-format']),
    title: fixture.tmux(['display-message', '-p', '-t', fixture.pane, '#{pane_title}']),
    position: fixture.tmux(['show-options', '-w', '-v', '-t', fixture.pane, 'pane-border-status']),
    format: fixture.tmux(['show-options', '-w', '-v', '-t', fixture.pane, 'pane-border-format']),
    style: fixture.tmux(['show-options', '-w', '-v', '-t', fixture.pane, 'pane-border-style']),
  };
}

function configureUserAppearance(fixture: E2EFixture): void {
  fixture.tmux(['select-pane', '-t', fixture.pane, '-T', 'original application title']);
  fixture.tmux(['set-option', '-w', '-t', fixture.pane, 'pane-border-status', 'bottom']);
  fixture.tmux(['set-option', '-w', '-t', fixture.pane, 'pane-border-format', USER_FORMAT]);
  fixture.tmux(['set-option', '-w', '-t', fixture.pane, 'pane-border-style', 'fg=green']);
}

describe('non-invasive pane badge presentation', { concurrent: false }, () => {
  it('prefixes only the bound pane, preserves shared formats, and renames without stacking', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
      const peer = fixture
        .tmux(['split-window', '-d', '-t', fixture.pane, '-P', '-F', '#{pane_id}', 'sleep 300'])
        .trim();
      const before = appearance(fixture);
      expect(localOption(fixture, 'pane-border-format')).toBe('');
      expectJsonResult(await fixture.runJsonCli(['name', 'alice']));
      const installed = `${BADGE_FRAGMENT}${USER_FORMAT}\n`;
      expect(localOption(fixture, 'pane-border-format')).toBe(installed);
      expect(localOption(fixture, BORDER_OWNER)).toBe(installed);
      expect(localOption(fixture, 'pane-border-format', peer)).toBe('');
      expect(localOption(fixture, BORDER_OWNER, peer)).toBe('');
      expect(appearance(fixture)).toEqual(before);
      const rendered = () =>
        fixture.tmux(['display-message', '-p', '-t', fixture.pane, '#{E:pane-border-format}']);
      expect(rendered()).toContain('[alice (tmt)]');
      expect(rendered()).toContain('repo/branch');
      expectJsonResult(await fixture.runJsonCli(['this', 'alice']));
      expectJsonResult(await fixture.runJsonCli(['identity', 'rename', 'alice', 'bob']));
      expect(badge(fixture)).toBe('bob (tmt)');
      expect(rendered()).toContain('[bob (tmt)]');
      expect(localOption(fixture, 'pane-border-format')).toBe(installed);
      expectJsonResult(await fixture.runJsonCli(['unbind']));
      expect(localOption(fixture, 'pane-border-format')).toBe('');
      expect(localOption(fixture, BORDER_OWNER)).toBe('');
      expect(appearance(fixture)).toEqual(before);
    });
  });

  it('preserves a local user override, including an explicitly empty one', async () => {
    for (const value of ['', 'user-local #{pane_index}']) {
      await withE2EFixture(async (fixture) => {
        configureUserAppearance(fixture);
        fixture.tmux(['set-option', '-p', '-t', fixture.pane, 'pane-border-format', value]);
        const before = appearance(fixture);
        expectJsonResult(await fixture.runJsonCli(['name', 'alice']));
        expect(localOption(fixture, 'pane-border-format')).toBe(`${value}\n`);
        expect(localOption(fixture, BORDER_OWNER)).toBe('');
        expectJsonResult(await fixture.runJsonCli(['unbind']));
        expect(localOption(fixture, 'pane-border-format')).toBe(`${value}\n`);
        expect(appearance(fixture)).toEqual(before);
      });
    }
  });

  it('does not duplicate an inherited badge reference or claim a user theme', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
      const integrated = `user-left ${BADGE_FRAGMENT} user-right`;
      fixture.tmux(['set-option', '-w', '-t', fixture.pane, 'pane-border-format', integrated]);
      const before = appearance(fixture);
      expectJsonResult(await fixture.runJsonCli(['name', 'alice']));
      expect(localOption(fixture, 'pane-border-format')).toBe('');
      expect(localOption(fixture, BORDER_OWNER)).toBe('');
      expectJsonResult(await fixture.runJsonCli(['unbind']));
      expect(appearance(fixture)).toEqual(before);
    });
  });

  it('clears its exact override on off, but retains a later user edit on cleanup', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
      const before = appearance(fixture);
      expectJsonResult(await fixture.runJsonCli(['name', 'alice']));
      expect(localOption(fixture, BORDER_OWNER)).not.toBe('');
      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'off', '--global'])
      );
      expectJsonResult(await fixture.runJsonCli(['this', 'alice']));
      expect(localOption(fixture, 'pane-border-format')).toBe('');
      expect(localOption(fixture, BORDER_OWNER)).toBe('');
      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'])
      );
      expectJsonResult(await fixture.runJsonCli(['this', 'alice']));
      fixture.tmux([
        'set-option',
        '-p',
        '-t',
        fixture.pane,
        'pane-border-format',
        'user edited after bind',
      ]);
      expectJsonResult(await fixture.runJsonCli(['unbind']));
      expect(localOption(fixture, 'pane-border-format')).toBe('user edited after bind\n');
      expect(badge(fixture)).toBe('');
      expect(appearance(fixture)).toEqual(before);
    });
  });

  it('keeps the committed binding successful when border publication is denied', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
      const before = appearance(fixture);
      const wrapper = path.join(fixture.wrapperDir, 'tmux');
      const denied = path.join(fixture.root, 'border-denied.log');
      writeExecutable(
        wrapper,
        fs
          .readFileSync(wrapper, 'utf8')
          .replace(
            'metadata_write=0\n',
            `for argument in "$@"; do\n  if [ "$argument" = "pane-border-format" ]; then\n    for command in "$@"; do\n      if [ "$command" = "set-option" ]; then printf 'denied\\n' >> '${denied}'; exit 1; fi\n    done\n  fi\ndone\nmetadata_write=0\n`
          ),
        0o755
      );
      expectJsonResult(await fixture.runJsonCli(['name', 'alice']));
      expect(durableState(fixture).bindings).toHaveLength(1);
      expect(badge(fixture)).toBe('alice (tmt)');
      expect(localOption(fixture, 'pane-border-format')).toBe('');
      expect(localOption(fixture, BORDER_OWNER)).toBe('');
      expectJsonResult(await fixture.runJsonCli(['unbind']));
      expect(durableState(fixture).bindings).toHaveLength(0);
      expect(fs.readFileSync(denied, 'utf8')).toBe('denied\n');
      expect(appearance(fixture)).toEqual(before);
    });
  });

  it('prints one enable-command hint when window borders are off without enabling them', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
      fixture.tmux(['set-option', '-w', '-t', fixture.pane, 'pane-border-status', 'off']);
      const before = appearance(fixture);
      const result = await fixture.runJsonCli(['name', 'alice']);
      expect(result.code).toBe(0);
      expect(result.json).toMatchObject({ name: 'alice' });
      expect(result.stderr.trim().split('\n')).toHaveLength(1);
      expect(result.stderr).toContain('hint: pane borders are off; enable them with tmux');
      expect(result.stderr).toContain(`set-option -w -t ${fixture.pane} pane-border-status top`);
      expect(badge(fixture)).toBe('alice (tmt)');
      expect(localOption(fixture, 'pane-border-format')).toBe(`${BADGE_FRAGMENT}${USER_FORMAT}\n`);
      expect(appearance(fixture)).toEqual(before);
      expectJsonResult(await fixture.runJsonCli(['unbind']));
      expect(appearance(fixture)).toEqual(before);
    });
  });

  it('keeps hook and talk recovery refreshes silent when window borders are off', async () => {
    await withE2EFixture(async (fixture) => {
      const shell = fixture.createShellPane('silent-badge');
      const pane = shell.pane;
      fixture.tmux(['set-option', '-w', '-t', pane, 'pane-border-status', 'off']);
      const scenario = path.join(fixture.root, 'silent-badge-scenario.json');
      const report = path.join(fixture.root, 'silent-badge-report.json');
      const nextScenario = path.join(fixture.root, 'silent-badge-next.json');
      const nextReport = path.join(fixture.root, 'silent-badge-next-report.json');
      const checkpoint = path.join(fixture.root, 'silent-badge-held');
      fs.writeFileSync(
        scenario,
        JSON.stringify([
          { args: ['name', 'Silent Badge', '-s', '--json'] },
          {
            args: ['__hook', 'claude', '--worker', '--work-budget-ms', '2000'],
            input: {
              hook_event_name: 'SessionStart',
              session_id: '55555555-5555-4555-8555-555555555555',
              source: 'startup',
            },
          },
        ])
      );
      fs.writeFileSync(nextScenario, JSON.stringify([{ args: ['whoami', '--json'], checkpoint }]));
      const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
      const launch = (input: string, output: string, listen: boolean) => {
        const command = [
          'env',
          `TMT_HOME=${fixture.globalDir}`,
          '/opt/tmt-tests/claude',
          fixture.executables.cli.executable,
          input,
          output,
          ...(listen ? ['--listen'] : []),
        ]
          .map(quote)
          .join(' ');
        fixture.tmux(['send-keys', '-t', pane, '-l', command]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
      };
      launch(scenario, report, false);
      const results = JSON.parse(await waitForFileContent(report, { timeoutMs: 15000 })) as Array<{
        code: number;
        stdout: string;
        stderr: string;
        badge: string;
      }>;
      const running = '#[push-default]#[fg=green]●#[default]#[pop-default] Silent Badge (tmt)';
      expect(results[0].code).toBe(0);
      expect(results[0].stderr).toContain('hint: pane borders are off');
      expect(results[1].code).toBe(0);
      expect(results[1].stdout).toContain('Silent Badge');
      expect(results[1].stderr).toBe('');
      expect(results[1].badge).toBe(running);
      const readBadge = () =>
        fixture.tmux(['-u', 'show-options', '-p', '-qv', '-t', pane, BADGE_OPTION]).trim();
      const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), {
        readonly: true,
      });
      try {
        const readRuntime = () =>
          database
            .prepare('SELECT runtime_state, runtime_pid FROM bindings WHERE pane_id=?')
            .get(pane) as { runtime_state: string; runtime_pid: number };
        const before = readRuntime();
        expect(before.runtime_state).toBe('running');
        // Let the hook-admitted native runtime actually exit. The next runtime
        // waits before calling TMT, so only talk can admit its replacement.
        await fixture.waitFor(
          () =>
            !fs.existsSync(`/proc/${before.runtime_pid}`) &&
            fixture
              .tmux(['display-message', '-p', '-t', pane, '#{pane_pid}|#{pane_current_command}'])
              .trim() === `${shell.pid}|bash`,
          5000,
          'original provider exited and the same pane returned to its shell'
        );
        launch(nextScenario, nextReport, true);
        await fixture.waitFor(() => fs.existsSync(checkpoint), 15000, 'replacement provider held');
        expect(readRuntime()).toEqual(before);
        fixture.tmux(['set-option', '-p', '-u', '-t', pane, BADGE_OPTION]);
        fixture.tmux(['set-option', '-p', '-u', '-t', pane, 'pane-border-format']);
        fixture.tmux(['set-option', '-p', '-u', '-t', pane, BORDER_OWNER]);
        const talk = await fixture.runJsonCli([
          'talk',
          'Silent Badge',
          'quiet refresh',
          '--detach',
        ]);
        expectJsonResult(talk);
        expect(readBadge()).toBe(running);
        expect(localOption(fixture, BORDER_OWNER, pane)).not.toBe('');
        const after = readRuntime();
        expect(after.runtime_state).toBe('running');
        expect(after.runtime_pid).toBeGreaterThan(0);
        expect(after.runtime_pid).not.toBe(before.runtime_pid);
        expect(fs.existsSync(`/proc/${after.runtime_pid}`)).toBe(true);
      } finally {
        database.close();
      }
      fs.writeFileSync(checkpoint, 'continue');
      const resumed = JSON.parse(
        await waitForFileContent(nextReport, { timeoutMs: 15000 })
      ) as Array<{
        code: number;
        stderr: string;
      }>;
      expect(resumed[0].code).toBe(0);
      expect(resumed[0].stderr).toBe('');
      expect(
        fixture.tmux(['show-options', '-w', '-v', '-t', pane, 'pane-border-status']).trim()
      ).toBe('off');
    });
  });
  it('updates recorded run state without changing the theme and clears on the next transition when off', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('badge-run').pane;
      const readBadge = () =>
        fixture.tmux(['-u', 'show-options', '-p', '-qv', '-t', pane, BADGE_OPTION]).trim();
      const format = `left ${BADGE_FRAGMENT} right`;
      fixture.tmux(['set-option', '-w', '-t', pane, 'pane-border-format', format]);
      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'])
      );
      expectJsonResult(await fixture.runJsonCli(['add', pane, 'State', '-s']));
      expect(readBadge()).toBe('State (tmt)');
      const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
      const run = async (suffix: string, disable: boolean) => {
        const done = path.join(fixture.root, `badge-${suffix}.done`);
        const command = [
          fixture.executables.cli.executable,
          ...fixture.executables.cli.args,
          'run',
          'State',
          '/bin/sleep',
          '300',
        ]
          .map(quote)
          .join(' ');
        fixture.tmux([
          'send-keys',
          '-t',
          pane,
          '-l',
          `${command}; printf '%s' "$?" > ${quote(done)}`,
        ]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        await fixture.waitFor(
          () => readBadge() === '#[push-default]#[fg=green]●#[default]#[pop-default] State (tmt)',
          5000,
          'running badge'
        );
        if (disable)
          expectJsonResult(
            await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'off', '--global'])
          );
        fixture.tmux(['send-keys', '-t', pane, 'C-c']);
        await fixture.waitFor(() => fs.existsSync(done), 5000, 'run completed');
        expect(fs.readFileSync(done, 'utf8')).toBe('130');
        expect(readBadge()).toBe(
          disable ? '' : '#[push-default]#[dim]○ State (tmt)#[default]#[pop-default]'
        );
        expect(
          durableState(fixture).bindings.find((row) => row.pane_id === pane)?.runtime_state
        ).toBe('ended');
        expect(
          fixture.tmux(['show-options', '-w', '-v', '-t', pane, 'pane-border-format']).trim()
        ).toBe(format);
      };
      await run('ended', false);
      await run('off', true);
    });
  });

  it('publishes the label by default without changing titles or shared window layout', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
      fixture.tmux(['new-session', '-d', '-t', 'e2e', '-s', 'grouped']);
      const before = appearance(fixture);
      expectJsonResult(await fixture.runJsonCli(['name', 'alice']));
      expect(badge(fixture)).toBe('alice (tmt)');
      expect(appearance(fixture)).toEqual(before);
      expectJsonResult(await fixture.runJsonCli(['unbind']));
      expect(badge(fixture)).toBe('');
      expect(appearance(fixture)).toEqual(before);
    });
  });

  it('preserves titles and shared window layout with the badge turned off', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'off', '--global'])
      );
      fixture.tmux(['new-session', '-d', '-t', 'e2e', '-s', 'grouped']);
      const before = appearance(fixture);
      expectJsonResult(await fixture.runJsonCli(['name', 'alice']));
      expect(badge(fixture)).toBe('');
      expect(appearance(fixture)).toEqual(before);
      expectJsonResult(await fixture.runJsonCli(['this', 'alice']));
      expectJsonResult(await fixture.runJsonCli(['unbind']));
      expect(appearance(fixture)).toEqual(before);
      expect(badge(fixture)).toBe('');
      expect(durableState(fixture).bindings).toHaveLength(0);
      expect(durableState(fixture).identities).toHaveLength(1);
    });
  });

  it('publishes only an opted-in label and applies config changes on the next binding', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
      // An existing user fragment takes precedence over automatic composition.
      const integrated = USER_FORMAT.replace('#[align=right]', `${BADGE_FRAGMENT}#[align=right]`);
      fixture.tmux(['set-option', '-w', '-t', fixture.pane, 'pane-border-format', integrated]);
      fixture.tmux(['new-session', '-d', '-s', 'independent', 'sleep 300']);
      fixture.tmux(['link-window', '-s', 'e2e:0', '-t', 'independent:']);
      const before = appearance(fixture);
      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'], {
          withoutTmux: true,
        })
      );
      expect(badge(fixture)).toBe('');
      expectJsonResult(await fixture.runJsonCli(['add', fixture.pane, 'alice']));
      expect(badge(fixture)).toBe('alice (tmt)');
      const rendered = fixture.tmux(['display-message', '-p', '-t', fixture.pane, integrated]);
      expect(rendered).toContain(' [alice (tmt)]');
      expect(rendered).toContain('repo/branch');
      expect(appearance(fixture)).toEqual(before);

      const metadata = fixture.paneMetadata();
      const bindings = durableState(fixture).bindings;
      const conflict = await fixture.runJsonCli(['add', fixture.pane, 'other']);
      expect(conflict.code).toBe(5);
      expect(conflict.json).toMatchObject({ error: { code: 'PANE_ALREADY_BOUND' } });
      expect(badge(fixture)).toBe('alice (tmt)');
      expect(fixture.paneMetadata()).toBe(metadata);
      expect(durableState(fixture).bindings).toEqual(bindings);

      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'off', '--global'], {
          withoutTmux: true,
        })
      );
      expect(badge(fixture)).toBe('alice (tmt)');
      expectJsonResult(await fixture.runJsonCli(['this', 'alice']));
      expect(badge(fixture)).toBe('');
      expect(
        fixture.tmux(['-u', 'display-message', '-p', '-t', fixture.pane, BADGE_FRAGMENT]).trim()
      ).toBe('');
      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'], {
          withoutTmux: true,
        })
      );
      expectJsonResult(await fixture.runJsonCli(['name', 'alice']));
      expect(badge(fixture)).toBe('alice (tmt)');
      expectJsonResult(await fixture.runJsonCli(['unbind']));
      expect(badge(fixture)).toBe('');
      expect(appearance(fixture)).toEqual(before);
    });
  });

  it('keeps format-like identity names literal in the cosmetic label', async () => {
    await withE2EFixture(async (fixture) => {
      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'])
      );
      const name = '#[fg=red]#{pane_title}#(false)';
      expectJsonResult(await fixture.runJsonCli(['name', name]));
      const identity = durableIdentity(fixture, name);
      expect(identity.lifetime).toBe('temporary');
      const label = '＃[fg=red]＃{pane_title}＃(false) (tmt)';
      expect(badge(fixture)).toBe(label);
      expect(
        fixture.tmux(['-u', 'display-message', '-p', '-t', fixture.pane, BADGE_FRAGMENT]).trim()
      ).toBe(`[${label}]`);
      expect(expectJsonResult(await fixture.runJsonCli(['whoami']))).toEqual({
        interfaceKind: 'container',
        sessionState: 'unknown',
        bound: true,
        id: identity.id,
        name,
        pane: fixture.pane,
        lifetime: 'temporary',
      });
    });
  });

  it('expands the documented width-limited fragment only for a bound, wide pane', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
      const before = appearance(fixture);
      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'])
      );
      expectJsonResult(await fixture.runJsonCli(['name', 'alice']));
      const rendered = () =>
        fixture.tmux(['display-message', '-p', '-t', fixture.pane, NARROW_BADGE_FRAGMENT]).trim();
      fixture.tmux(['resize-window', '-t', fixture.pane, '-x', '120']);
      expect(rendered()).toBe('[alice (tmt)]');
      fixture.tmux(['resize-window', '-t', fixture.pane, '-x', '60']);
      expect(rendered()).toBe('');
      fixture.tmux(['resize-window', '-t', fixture.pane, '-x', '120']);
      expectJsonResult(await fixture.runJsonCli(['unbind']));
      expect(rendered()).toBe('');
      expect(appearance(fixture)).toEqual(before);
    });
  });

  it('preserves successful binding and unbinding when badge writes are denied', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
      const before = appearance(fixture);
      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'])
      );
      const wrapper = path.join(fixture.wrapperDir, 'tmux');
      const denied = path.join(fixture.root, 'badge-denied.log');
      writeExecutable(
        wrapper,
        fs
          .readFileSync(wrapper, 'utf8')
          .replace(
            '#!/bin/sh\n',
            `#!/bin/sh\nfor argument in "$@"; do\n  if [ "$argument" = "@tmt.badge" ]; then printf 'denied\\n' >> '${denied}'; exit 1; fi\ndone\n`
          ),
        0o755
      );
      expectJsonResult(await fixture.runJsonCli(['name', 'alice']));
      const identity = durableIdentity(fixture, 'alice');
      expect(identity.lifetime).toBe('temporary');
      expect(expectJsonResult(await fixture.runJsonCli(['whoami']))).toEqual({
        interfaceKind: 'container',
        sessionState: 'unknown',
        bound: true,
        id: identity.id,
        name: 'alice',
        pane: fixture.pane,
        lifetime: 'temporary',
      });
      expect(durableState(fixture).bindings).toHaveLength(1);
      expectJsonResult(await fixture.runJsonCli(['unbind']));
      expect(durableState(fixture).bindings).toHaveLength(0);
      expect(fs.readFileSync(denied, 'utf8').trim().split('\n')).toEqual(['denied', 'denied']);
      expect(appearance(fixture)).toEqual(before);
    });
  });

  it('rejects invalid badge settings before binding but still permits unbinding', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
      const before = appearance(fixture);
      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'])
      );
      expectJsonResult(await fixture.runJsonCli(['name', 'alice']));
      const committed = durableState(fixture);
      const metadata = fixture.paneMetadata();
      const configPath = path.join(fixture.globalDir, 'config.json');
      fs.writeFileSync(configPath, JSON.stringify({ ui: { paneBadge: 'invalid' } }));

      for (const command of [
        ['name', 'alice'],
        ['this', 'alice'],
        ['add', fixture.pane, 'alice'],
      ]) {
        const rejected = await fixture.runJsonCli(command);
        expect(rejected.code).toBe(1);
        expect(rejected.json).toMatchObject({ error: { code: 'CONFIG_ERROR' } });
        expect(durableState(fixture)).toEqual(committed);
        expect(fixture.paneMetadata()).toBe(metadata);
        expect(badge(fixture)).toBe('alice (tmt)');
      }

      expectJsonResult(await fixture.runJsonCli(['unbind']));
      expect(durableState(fixture).bindings).toHaveLength(0);
      expect(badge(fixture)).toBe('');
      expect(appearance(fixture)).toEqual(before);
    });
  });
});
