import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';

/** Puts the built squad extension and a `tmt` launcher on the fixture PATH. */
function installSquad(fixture: E2EFixture): void {
  const cli = resolveCliExecutables().cli.executable;
  const squad = path.join(path.dirname(cli), 'tmt-squad');
  if (!fs.existsSync(squad)) throw new Error(`Build tmt-squad first: ${squad}`);
  fs.symlinkSync(squad, path.join(fixture.wrapperDir, 'tmt-squad'));
  fs.symlinkSync(cli, path.join(fixture.wrapperDir, 'tmt'));
}

/** Extension options follow the extension name, so `--json` goes last. */
function squadCli<T = Record<string, unknown>>(fixture: E2EFixture, args: string[]) {
  return fixture.runCli<T>(['squad', ...args, '--json']);
}

function errorCode(result: { json?: unknown }): string | undefined {
  return (result.json as { error?: { code?: string } } | undefined)?.error?.code;
}

/** Noted prefix bindings as `key|note` (tmux 3.3 has no `list-keys -F`). */
function notedKeys(fixture: E2EFixture): string[] {
  return fixture
    .tmux(['list-keys', '-N', '-P', '', '-T', 'prefix'])
    .split('\n')
    .map((line) => line.trim().replace(/\s+/, '|'))
    .filter(Boolean);
}

/** The command bound to a prefix key, if any. */
function boundCommand(fixture: E2EFixture, key: string): string | undefined {
  return fixture
    .tmux(['list-keys', '-T', 'prefix'])
    .split('\n')
    .map((line) =>
      line
        .split('-T prefix')[1]
        ?.trim()
        .split(/\s+(.*)/)
    )
    .find((parts) => parts?.[0] === key)?.[1];
}

function clientPlace(fixture: E2EFixture): string {
  return fixture.tmux(['list-clients', '-F', '#{session_name}/#{pane_id}']).trim();
}

async function squadWithMember(fixture: E2EFixture): Promise<string> {
  installSquad(fixture);
  const member = fixture
    .tmux(['new-session', '-d', '-s', 'crew', '-P', '-F', '#{pane_id}', 'cat'])
    .trim();
  expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Ben']));
  expectJsonResult(await fixture.runJsonCli(['add', member, 'auth-fix']));
  expectJsonResult(await squadCli(fixture, ['init', 'product', '--me', 'Ben']));
  expectJsonResult(await squadCli(fixture, ['add', 'auth-fix']));
  return member;
}

describe('squad on a private tmux server', { concurrent: false }, () => {
  it('loads hotkeys into the running server, refuses a taken key, and unbinds only its own', async () => {
    await withE2EFixture(async (fixture) => {
      await squadWithMember(fixture);
      const conf = path.join(fixture.root, 'tmux.conf');
      fs.writeFileSync(conf, 'set -g mouse on\n');
      const install = ['hotkeys', 'install', '--yes', '--config', conf];

      fixture.tmux(['bind-key', 'S', 'choose-tree']);
      const taken = await squadCli(fixture, install);
      expect(errorCode(taken)).toBe('SQUAD_HOTKEY_TAKEN');
      expect(JSON.stringify(taken.json)).toContain('running server');
      expect(fs.readFileSync(conf, 'utf8')).toBe('set -g mouse on\n');

      fixture.tmux(['unbind-key', 'S']);
      expectJsonResult(await squadCli(fixture, install));
      expect(notedKeys(fixture)).toEqual(
        expect.arrayContaining(['S|tmt squad popup', 'B|tmt squad pane'])
      );
      expect(boundCommand(fixture, 'S')).toContain('squad board --popup');
      expect(fs.readFileSync(conf, 'utf8')).toMatch(
        /^set -g mouse on\nsource-file -q '.*squad\.tmux\.conf' # tmt squad hotkeys\n$/
      );

      // The user rebinds B afterwards: removal must leave it alone.
      fixture.tmux(['bind-key', 'B', 'split-window']);
      const removed = expectJsonResult<{ unbound: string[] }>(
        await squadCli<{ unbound: string[] }>(fixture, ['hotkeys', 'remove', '--yes'])
      );
      expect(removed.unbound).toEqual(['S']);
      expect(boundCommand(fixture, 'S')).toBeUndefined();
      expect(boundCommand(fixture, 'B')).toBe('split-window');
    });
  });

  it('acts as the calling pane’s identity, and as the recorded user from an unnamed pane', async () => {
    await withE2EFixture(async (fixture) => {
      await squadWithMember(fixture);
      const lead = fixture.createShellPane('lead');
      expectJsonResult(await fixture.runJsonCli(['add', '--save', lead.pane, 'Sol']));
      const unnamed = fixture.createShellPane('unnamed');
      const talk = async (pane: string, file: string) => {
        const out = path.join(fixture.root, file);
        fixture.tmux([
          'send-keys',
          '-t',
          pane,
          `tmt squad annotate auth-fix 'rebase first' --to member --json > '${out}'; echo TALK_EXIT=$?`,
          'Enter',
        ]);
        await fixture.waitForCapture((screen) => screen.includes('TALK_EXIT=0'), pane);
        return JSON.parse(fs.readFileSync(out, 'utf8')) as { as: string };
      };
      // me is Ben, yet the lead's own pane speaks as the lead.
      expect((await talk(lead.pane, 'lead.json')).as).toBe('Sol');
      expect((await talk(unnamed.pane, 'unnamed.json')).as).toBe('Ben');
    });
  });

  it('closes a --popup board after its jump and keeps the pane board open', async () => {
    await withE2EFixture(async (fixture) => {
      const member = await squadWithMember(fixture);
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      const start = clientPlace(fixture);
      expect(start).toBe(`e2e/${shell.pane}`);

      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'tmt squad board --popup; echo BOARD_EXIT=$?',
        'Enter',
      ]);
      await fixture.waitForCapture((screen) => screen.includes('auth-fix'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitForCapture((screen) => screen.includes('BOARD_EXIT=0'), shell.pane);
      expect(clientPlace(fixture)).toBe(`crew/${member}`);

      // The pane form: the same jump leaves the board running.
      fixture.tmux(['switch-client', '-t', shell.pane]);
      fixture.tmux(['send-keys', '-t', shell.pane, 'clear; tmt squad board', 'Enter']);
      await fixture.waitForCapture(
        (screen) => screen.includes('auth-fix') && !screen.includes('BOARD_EXIT'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitFor(() => clientPlace(fixture) === `crew/${member}`, 5_000, 'jumped');
      await new Promise((resolve) => setTimeout(resolve, 500));
      expect(fixture.capture(40, shell.pane)).toContain('auth-fix');
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture((screen) => !screen.includes('MEMBER'), shell.pane);
    });
  });
});
