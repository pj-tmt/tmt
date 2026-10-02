import { writeExecutable } from '../support/executable-fixture.mjs';
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { durableIdentity, durableState } from './identity-state-oracle.js';

const BADGE_OPTION = '@tmux-team.badge';
const USER_FORMAT = '#[align=left]#{window_index}.#{pane_index}#[align=right]repo/branch';
const BADGE_FRAGMENT = '#{?@tmux-team.badge, [#{@tmux-team.badge}],}';
const NARROW_BADGE_FRAGMENT =
  '#{?#{&&:#{@tmux-team.badge},#{e|>=:#{pane_width},80}}, [#{@tmux-team.badge}],}';

function badge(fixture: E2EFixture): string {
  // The minimal Docker image has no UTF-8 locale. Request UTF-8 output so tmux
  // does not render wide characters as underscores in this read-side oracle.
  return fixture.tmux(['-u', 'show-options', '-p', '-qv', '-t', fixture.pane, BADGE_OPTION]).trim();
}

function appearance(fixture: E2EFixture) {
  return {
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

  it('preserves titles and shared window layout with the default-off badge', async () => {
    await withE2EFixture(async (fixture) => {
      configureUserAppearance(fixture);
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
      // The user, not TMT, chooses where to insert the fragment in their theme.
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
            `#!/bin/sh\nfor argument in "$@"; do\n  if [ "$argument" = "@tmux-team.badge" ]; then printf 'denied\\n' >> '${denied}'; exit 1; fi\ndone\n`
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
