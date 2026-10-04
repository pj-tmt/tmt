import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { spawnRealTmuxCli, releaseRealTmuxCli, readRealTmuxCli } from './real-tmux-caller.js';
import { describe, expect, it } from 'vite-plus/test';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import { expectJsonResult } from './cli-assertions.js';
import { durableIdentity, durableState } from './identity-state-oracle.js';
import { requestAttempts } from './request-state-oracle.js';
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
  it('sends a scheduled slot once, refuses a second clock and releases its lease on shutdown', async () => {
    await withE2EFixture(
      async (fixture) => {
        installSquad(fixture);
        const home = path.join(fixture.root, 'clock-home');
        fs.mkdirSync(home);
        fixture.tmux(['set-environment', '-g', 'HOME', home]);
        fixture.tmux(['set-environment', '-g', 'XDG_CACHE_HOME', path.join(home, 'cache')]);
        fixture.tmux(['set-environment', '-g', 'TMUX_TEAM_HOME', fixture.globalDir]);
        expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Ben']));
        const peer = await fixture.createMockPane('clock-owner');
        expectJsonResult(await fixture.runJsonCli(['add', '--save', peer.pane, 'worker']));
        const owner = durableIdentity(fixture, 'worker').id;
        expectJsonResult(await squadCli(fixture, ['init', 'product', '--me', 'Ben']));
        expectJsonResult(await squadCli(fixture, ['add', 'worker']));
        const leasePath = path.join(fixture.globalDir, 'squad', 'cron', 'clock.json');
        const clock = await spawnRealTmuxCli(fixture, ['squad', 'cron', 'run', '--json'], {
          name: 'primary-clock',
          json: false,
        });
        fs.writeFileSync(clock.releasePath, 'run');
        await fixture.waitFor(() => fs.existsSync(leasePath), 5_000, 'clock lease publication');
        const holder = JSON.parse(fs.readFileSync(leasePath, 'utf8'));
        expect(holder.pane).toBe(clock.pane);
        const second = await spawnRealTmuxCli(fixture, ['squad', 'cron', 'run', '--json'], {
          name: 'second-clock',
          json: false,
        });
        await releaseRealTmuxCli(fixture, second);
        expect(readRealTmuxCli<{ error: { code: string } }>(second)).toMatchObject({
          code: 1,
          stdout: { error: { code: 'SQUAD_CRON_CLOCK_RUNNING' } },
        });
        const message = 'literal {time}\nclock reminder';
        const added = expectJsonResult<{ job: { roomId: string } }>(
          await squadCli(fixture, [
            'cron',
            'add',
            'product',
            'worker',
            '--identity',
            'Ben',
            '--every',
            '1m',
            message,
          ])
        );
        const scheduled = () =>
          requestAttempts(fixture).filter((row) => row.message_text === message);
        await fixture.waitFor(
          () => scheduled().length === 1,
          5_000,
          'scheduled request acceptance'
        );
        const request = scheduled()[0]!;
        expect(request).toMatchObject({
          recipient_identity_id: owner,
          originator_kind: 'unknown',
          originator_identity_id: null,
          room_id: added.job.roomId,
          route_kind: 'inbox',
          message_text: message,
        });
        await fixture.waitForEvent(
          (event) =>
            event.event === 'input' &&
            event.pid === peer.pid &&
            event.line?.startsWith(`[tmt] request ${request.request_id} is queued:`) === true,
          5_000
        );
        const busy = await squadCli(fixture, ['cron', 'tick']);
        expect(busy.code).toBe(1);
        expect(errorCode(busy)).toBe('SQUAD_CRON_CLOCK_RUNNING');
        process.kill(holder.pid, 'SIGTERM');
        await fixture.waitFor(() => fs.existsSync(clock.exitPath), 5_000, 'clock signal cleanup');
        expect(readRealTmuxCli(clock).code).toBe(0);
        expect(fs.existsSync(leasePath)).toBe(false);
        expect(fixture.mockProcessIsRunning(holder.pid)).toBe(false);
        for (let attempt = 0; attempt < 2; attempt++) {
          expectJsonResult(await squadCli(fixture, ['cron', 'tick']));
        }
        expect(scheduled()).toHaveLength(1);
        expect(
          fixture
            .events()
            .filter(
              (event) =>
                event.event === 'input' &&
                event.pid === peer.pid &&
                event.line?.startsWith(`[tmt] request ${request.request_id} is queued:`)
            )
        ).toHaveLength(1);
        const status = expectJsonResult<{ clock: { state: string } }>(
          await squadCli(fixture, ['cron', 'clock'])
        );
        expect(status.clock.state).toBe('no clock');
        expect(fs.existsSync(leasePath)).toBe(false);
      },
      { mode: 'input-log' }
    );
  }, 30_000);

  it('queues cron announcements for the right owners and reconciles retirement in isolated storage', async () => {
    await withE2EFixture(async (fixture) => {
      installSquad(fixture);
      const home = path.join(fixture.root, 'cron-home');
      fs.mkdirSync(home);
      fixture.tmux(['set-environment', '-g', 'HOME', home]);
      fixture.tmux(['set-environment', '-g', 'XDG_CACHE_HOME', path.join(home, 'cache')]);
      fixture.tmux(['set-environment', '-g', 'TMUX_TEAM_HOME', fixture.globalDir]);
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Ben']));
      const ownerPane = fixture.createShellPane('cron-owner');
      const leadPane = fixture.createShellPane('cron-lead');
      expectJsonResult(await fixture.runJsonCli(['add', '--save', ownerPane.pane, 'worker']));
      expectJsonResult(await fixture.runJsonCli(['add', '--save', leadPane.pane, 'Sol']));
      const owner = durableIdentity(fixture, 'worker').id;
      const lead = durableIdentity(fixture, 'Sol').id;
      expectJsonResult(await squadCli(fixture, ['init', 'product', '--me', 'Ben']));
      expectJsonResult(await squadCli(fixture, ['lead', 'Sol']));
      expectJsonResult(await squadCli(fixture, ['add', 'worker']));
      let commandNumber = 0;
      const cron = async (args: string[]) => {
        const process = await spawnRealTmuxCli(fixture, ['squad', 'cron', ...args, '--json'], {
          name: `cron-${commandNumber++}`,
          json: false,
        });
        await releaseRealTmuxCli(fixture, process);
        const result = readRealTmuxCli<Record<string, unknown>>(process);
        return {
          code: result.code,
          stdout: JSON.stringify(result.stdout),
          stderr: result.stderr,
          json: result.stdout,
        };
      };
      const observeNotices = () => {
        const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
        try {
          return db
            .prepare(
              'SELECT recipient_identity_id AS recipient, request_kind AS kind, message_text AS message FROM request_attempts ORDER BY rowid'
            )
            .all() as { recipient: string; kind: string; message: string }[];
        } finally {
          db.close();
        }
      };
      expectJsonResult(
        await cron([
          'add',
          'product',
          'worker',
          '--identity',
          'Ben',
          '--every',
          '1h',
          'literal {time} reminder',
        ])
      );
      expect(observeNotices()).toHaveLength(1);
      expectJsonResult(await cron(['pause', 'product', 'c1', '--identity', 'Ben']));
      expect(observeNotices().at(-1)).toMatchObject({
        recipient: owner,
        message: expect.stringContaining('paused by Ben'),
      });
      expectJsonResult(await cron(['resume', 'product', 'c1', '--identity', 'Ben']));
      expect(observeNotices().at(-1)).toMatchObject({
        recipient: owner,
        message: expect.stringContaining('resumed by Ben'),
      });
      expectJsonResult(await cron(['reassign', 'product', 'c1', 'Sol', '--identity', 'Ben']));
      expect(observeNotices().slice(-2)).toMatchObject([
        { recipient: owner },
        { recipient: lead, message: expect.stringContaining('literal {time} reminder') },
      ]);
      const count = observeNotices().length;
      expectJsonResult(await cron(['pause', 'product', 'c1', '--identity', 'Sol']));
      expect(observeNotices()).toHaveLength(count);
      expectJsonResult(await cron(['reassign', 'product', 'c1', 'worker', '--identity', 'Ben']));
      expectJsonResult(await fixture.runJsonCli(['rm', 'worker', '--force']));
      const retired = expectJsonResult(await cron(['show', 'product', 'c1']));
      expect(retired.job).toMatchObject({ state: 'no owner', ownerId: null });
      expect(observeNotices().at(-1)).toMatchObject({
        recipient: lead,
        message: expect.stringContaining('worker retired'),
      });
      const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
      try {
        expect(
          db
            .prepare(
              "SELECT state FROM identity_hooks WHERE consumer='squad-cron' AND identity_id=?"
            )
            .get(owner)
        ).toEqual({ state: 'delivered' });
      } finally {
        db.close();
      }
      const after = observeNotices().length;
      expectJsonResult(await cron(['ls']));
      expect(observeNotices()).toHaveLength(after);
      for (const notice of observeNotices()) {
        expect(notice.kind).toBe('announcement');
        expect(notice.message.startsWith('▚ ⏱')).toBe(true);
      }
    });
  });

  it('drops a lost temporary member on the first read and retains a saved member offline', async () => {
    await withE2EFixture(async (fixture) => {
      const temporaryPane = await squadWithMember(fixture);
      const savedPane = fixture.createShellPane('saved');
      expectJsonResult(await fixture.runJsonCli(['add', '--save', savedPane.pane, 'saved-worker']));
      expectJsonResult(await squadCli(fixture, ['add', 'saved-worker']));
      const temporary = durableIdentity(fixture, 'auth-fix');
      const saved = durableIdentity(fixture, 'saved-worker');
      expect(temporary.lifetime).toBe('temporary');
      expect(saved.lifetime).toBe('saved');
      fixture.tmux(['kill-pane', '-t', temporaryPane]);
      fixture.tmux(['kill-pane', '-t', savedPane.pane]);
      // Read-only SQL must observe the pre-reconciliation state, not trigger cleanup.
      expect(durableIdentity(fixture, 'auth-fix').retired_at_ms).toBeNull();
      const first = expectJsonResult<{
        sections: { rows: { name: string; presence: string }[] }[];
      }>(await squadCli(fixture, ['ls', '--squad', 'product']));
      expect(first.sections.flatMap((section) => section.rows)).toMatchObject([
        { name: 'saved-worker', presence: 'offline' },
      ]);
      expect(first.sections.flatMap((section) => section.rows)).toHaveLength(1);
      const after = durableState(fixture);
      expect(after.identities.find((row) => row.id === temporary.id)?.retired_at_ms).toEqual(
        expect.any(Number)
      );
      expect(after.identities.find((row) => row.id === saved.id)?.retired_at_ms).toBeNull();
      expect(
        after.bindings.filter((row) => [temporary.id, saved.id].includes(String(row.identity_id)))
      ).toEqual([]);
    });
  });

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
