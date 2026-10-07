import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { spawnRealTmuxCli, releaseRealTmuxCli, readRealTmuxCli } from './real-tmux-caller.js';
import { describe, expect, it } from 'vite-plus/test';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import { expectJsonResult } from './cli-assertions.js';
import { durableIdentity, durableState } from './identity-state-oracle.js';
import { requestAttempts } from './request-state-oracle.js';
import { withE2EFixture, type E2EFixture, type MockEvent } from './harness.js';

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
  it('starts the board with legacy jobs and stop instructions while an old clock defers Ops cutover', async () => {
    await withE2EFixture(async (fixture) => {
      installSquad(fixture);
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Ben']));
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'worker']));
      expectJsonResult(await squadCli(fixture, ['init', 'product', '--me', 'Ben']));
      expectJsonResult(await squadCli(fixture, ['add', 'worker']));
      const added = expectJsonResult<{ job: { id: string; message: string } }>(
        await squadCli(fixture, [
          'cron',
          'add',
          'product',
          'worker',
          '--every',
          '1h',
          'retained migration job',
        ])
      );
      const config = path.join(fixture.globalDir, 'ops.toml');
      const ops = path.join(fixture.globalDir, 'ops');
      const beforeConfig = fs.readFileSync(config);
      const beforeJobs = fs.readFileSync(path.join(ops, 'cron', 'jobs.json'));
      fs.renameSync(config, path.join(fixture.globalDir, 'squad.toml'));
      fs.renameSync(ops, path.join(fixture.globalDir, 'squad'));
      fs.unlinkSync(path.join(fixture.globalDir, '.ops-paths-v1'));
      fs.unlinkSync(path.join(fixture.globalDir, '.ops-paths-cutover-v1'));
      const lease = path.join(fixture.globalDir, 'squad', 'cron', 'clock.json');
      fs.writeFileSync(path.join(fixture.globalDir, 'squad', 'cron', 'clock.lock'), '');
      fs.writeFileSync(
        lease,
        JSON.stringify({
          version: 1,
          pid: 123,
          pane: '%41',
          sinceMs: Date.now() - 1000,
          expiresMs: Date.now() + 120_000,
        })
      );
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('pending-migration-board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'tmt squad board --squad product; echo MIGRATION_BOARD_EXIT=$?',
        'Enter',
      ]);
      await fixture.waitForCapture(
        (screen) =>
          screen.includes('product') && screen.includes('PID 123') && screen.includes('Ctrl-C'),
        shell.pane
      );
      expect(fs.existsSync(config)).toBe(false);
      expect(fs.existsSync(ops)).toBe(false);
      const pending = await squadCli<{ jobs: { id: string; message: string }[] }>(fixture, [
        'cron',
        'ls',
        '--squad',
        'product',
      ]);
      expect(pending.code, pending.stderr || pending.stdout).toBe(0);
      expect(pending.json).toBeDefined();
      expect(pending.stderr.match(/Ops migration deferred/g)).toHaveLength(1);
      expect(pending.stderr).toContain('PID 123 in pane %41');
      expect(pending.stderr).toContain('kill -TERM 123');
      expect(pending.json!.jobs[0]).toMatchObject(added.job);
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture(
        (screen) => screen.includes('MIGRATION_BOARD_EXIT=0'),
        shell.pane
      );
      fs.unlinkSync(lease);
      const migrated = expectJsonResult<{ jobs: { id: string; message: string }[] }>(
        await squadCli(fixture, ['cron', 'ls', '--squad', 'product'])
      );
      expect(migrated.jobs).toEqual(pending.json!.jobs);
      expect(fs.readFileSync(config)).toEqual(beforeConfig);
      expect(fs.readFileSync(path.join(ops, 'cron', 'jobs.json'))).toEqual(beforeJobs);
      expect(fs.existsSync(path.join(fixture.globalDir, 'squad'))).toBe(false);
    });
  });

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
        const leasePath = path.join(fixture.globalDir, 'ops', 'cron', 'clock.json');
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
        // Correlate the peer's consumed wake with durable request identity, not notice paint.
        const isScheduledWake = (event: MockEvent) =>
          event.event === 'input' &&
          event.pid === peer.pid &&
          event.line?.split(/\s+/).includes(request.request_id) === true;
        await fixture.waitForEvent(isScheduledWake, 5_000);
        await fixture.waitFor(
          () => scheduled()[0]?.wake_state === 'sent',
          5_000,
          'scheduled wake committed'
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
        expect(fixture.events().filter(isScheduledWake)).toHaveLength(1);
        expect(scheduled()[0]).toMatchObject({
          request_id: request.request_id,
          wake_state: 'sent',
        });
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

  it('uses comma Actions for an empty room and persists explicit checklist updates without dispatch', async () => {
    await withE2EFixture(async (fixture) => {
      installSquad(fixture);
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Ben']));
      expectJsonResult(await squadCli(fixture, ['init', 'product', '--me', 'Ben']));
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('checklist-board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      const config = fs.readFileSync(path.join(fixture.globalDir, 'ops.toml'));
      const state = durableState(fixture);
      const beforeRequests = requestAttempts(fixture);
      {
        fixture.tmux([
          'send-keys',
          '-t',
          shell.pane,
          'tmt squad board --squad product; echo CHECKLIST_BOARD_EXIT=$?',
          'Enter',
        ]);
        await fixture.waitForCapture(
          (screen) => screen.includes('product') && screen.includes('(no members)'),
          shell.pane
        );
        fixture.tmux(['send-keys', '-t', shell.pane, ',']);
        await fixture.waitForCapture((screen) => screen.includes('Actions…'), shell.pane);
        fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
        await fixture.waitForCapture(
          (screen) => screen.includes('Checklist') && screen.includes('Esc closes'),
          shell.pane
        );
        fixture.tmux(['send-keys', '-t', shell.pane, ...Array(20).fill('Down'), 'Enter']);
        await fixture.waitForCapture((screen) => screen.includes('No checklist yet'), shell.pane);
        fixture.tmux(['send-keys', '-t', shell.pane, 'Tab', 'Tab', 'Enter']);
        await fixture.waitForCapture((screen) => screen.includes('Create item'), shell.pane);
        fixture.tmux(['send-keys', '-t', shell.pane, 'Down', 'Enter']);
        await fixture.waitForCapture((screen) => screen.includes('Authored fields'), shell.pane);
        fixture.tmux(['send-keys', '-l', '-t', shell.pane, 'Native board item']);
        fixture.tmux(['send-keys', '-t', shell.pane, 'End', 'Enter']);
        await fixture.waitForCapture(
          (screen) => screen.includes('Review exact update') && screen.includes('inventory Absent'),
          shell.pane
        );
        const shown = expectJsonResult(
          await fixture.runJsonCli<{ room: { id: string } }>(['room', 'show', 'squad-product'])
        );
        const roomId = shown.room.id;
        const file = path.join(fixture.globalDir, 'ops', 'checklist', roomId, 'items.json');
        expect(fs.existsSync(file)).toBe(false);
        fixture.tmux(['send-keys', '-t', shell.pane, 'Down', 'Enter']);
        await fixture.waitFor(() => fs.existsSync(file), 5_000, 'checklist publication');
        let stored = JSON.parse(fs.readFileSync(file, 'utf8'));
        expect(stored.items).toHaveLength(1);
        expect(stored.items[0]).toMatchObject({
          title: 'Native board item',
          completion: 'open',
          archived: false,
          assignee: null,
          revision: 1,
        });
        await fixture.waitForCapture((screen) => screen.includes('acknowledged'), shell.pane);
        fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
        await fixture.waitForCapture((screen) => screen.includes('Complete'), shell.pane);
        fixture.tmux(['send-keys', '-t', shell.pane, 'Tab', 'Down', 'Enter']);
        await fixture.waitForCapture(
          (screen) => screen.includes('Review exact update') && screen.includes('item revision 1'),
          shell.pane
        );
        fixture.tmux(['send-keys', '-t', shell.pane, 'Down', 'Enter']);
        await fixture.waitFor(
          () => JSON.parse(fs.readFileSync(file, 'utf8')).items[0].completion === 'complete',
          5_000,
          'explicit completion'
        );
        stored = JSON.parse(fs.readFileSync(file, 'utf8'));
        expect(stored.items[0].revision).toBe(2);
        expect(stored.inventoryRevision).toBe(1);
        expect(durableState(fixture)).toEqual(state);
        expect(requestAttempts(fixture)).toEqual(beforeRequests);
        expect(fs.readFileSync(path.join(fixture.globalDir, 'ops.toml'))).toEqual(config);
        await fixture.waitForCapture((screen) => screen.includes('hidden by filters'), shell.pane);
        fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
        await fixture.waitForCapture(
          (screen) => !screen.includes('Checklist') && screen.includes('(no members)'),
          shell.pane
        );
        fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
        await fixture.waitForCapture(
          (screen) => screen.includes('CHECKLIST_BOARD_EXIT=0'),
          shell.pane
        );
      }
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

      // A named squad opens on its own tab; an unnamed board opens on the home tab.
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'tmt squad board --squad product --popup; echo BOARD_EXIT=$?',
        'Enter',
      ]);
      await fixture.waitForCapture((screen) => screen.includes('auth-fix'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitForCapture((screen) => screen.includes('BOARD_EXIT=0'), shell.pane);
      expect(clientPlace(fixture)).toBe(`crew/${member}`);

      // The pane form: the same jump leaves the board running.
      fixture.tmux(['switch-client', '-t', shell.pane]);
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'clear; tmt squad board --squad product',
        'Enter',
      ]);
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
