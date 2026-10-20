import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { spawnRealTmuxCli, releaseRealTmuxCli, readRealTmuxCli } from './real-tmux-caller.js';
import { describe, expect, it } from 'vite-plus/test';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import { expectJsonResult } from './cli-assertions.js';
import { durableIdentity, durableState } from './identity-state-oracle.js';
import { requestAttempts } from './request-state-oracle.js';
import { storedDocument } from '../support/ops-state.js';
import { withE2EFixture, type E2EFixture, type MockEvent } from './harness.js';

/** Puts the built squad extension and a `tmt` launcher on the fixture PATH. */
function installSquad(fixture: E2EFixture): void {
  const cli = resolveCliExecutables().cli.executable;
  const squad = path.join(path.dirname(cli), 'tmt-ops');
  if (!fs.existsSync(squad)) throw new Error(`Build tmt-ops first: ${squad}`);
  fs.symlinkSync(squad, path.join(fixture.wrapperDir, 'tmt-ops'));
  fs.symlinkSync(cli, path.join(fixture.wrapperDir, 'tmt'));
  // Board view hints are cached by HOME/XDG roots, independently of TMT_HOME.
  const home = path.join(fixture.root, 'ops-ui-home');
  fs.mkdirSync(home);
  fixture.tmux(['set-environment', '-g', 'HOME', home]);
  fixture.tmux(['set-environment', '-g', 'XDG_CACHE_HOME', path.join(home, 'cache')]);
}

/** Extension options follow the extension name, so `--json` goes last. */
function squadCli<T = Record<string, unknown>>(fixture: E2EFixture, args: string[]) {
  return fixture.runCli<T>(['ops', 'squad', ...args, '--json']);
}

function opsCli<T = Record<string, unknown>>(fixture: E2EFixture, args: string[]) {
  return fixture.runCli<T>(['ops', ...args, '--json']);
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

/** Actual pane slots, independent of the board's selected-row/model state. */
function hostPaneSlots(fixture: E2EFixture): Record<string, string> {
  return Object.fromEntries(
    fixture
      .tmux([
        'list-panes',
        '-a',
        '-F',
        '#{pane_id}|#{window_id}|#{pane_left}|#{pane_top}|#{pane_width}|#{pane_height}',
      ])
      .trim()
      .split('\n')
      .map((line) => {
        const [pane, ...slot] = line.split('|');
        return [pane!, slot.join('|')];
      })
  );
}

/** Select only after the committed filter has produced its current row geometry. */
async function selectHostWorker(fixture: E2EFixture, board: string) {
  await fixture.waitForCapture((screen) => /\/[ \t]+Search/.test(screen), board);
  fixture.tmux(['send-keys', '-t', board, '/', 'worker', 'Enter']);
  await fixture.waitForCapture(
    (screen) => screen.includes('/ worker') && screen.includes('1 of 2 agents'),
    board
  );
  fixture.tmux(['send-keys', '-t', board, 'End']);
  await fixture.waitForCapture((screen) => screen.includes('⏎ to message worker'), board);
}

/** Two consumers share the same real board/caller/lead/member setup. */
async function hostActionBoard(fixture: E2EFixture, action: string, task: string) {
  installSquad(fixture);
  const left = fixture.createShellPane('host-left');
  const worker = fixture.createShellPane('host-worker');
  const board = fixture.createShellPane('host-board');
  expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Ben']));
  // Leadership deliberately uses a saved identity; the worker stays temporary for pane-gone coverage.
  expectJsonResult(await fixture.runJsonCli(['add', '--save', left.pane, 'left-agent']));
  expectJsonResult(await fixture.runJsonCli(['add', worker.pane, 'worker']));
  expectJsonResult(await squadCli(fixture, ['init', 'product', '--me', 'Ben']));
  expectJsonResult(await squadCli(fixture, ['lead', 'left-agent']));
  expectJsonResult(await squadCli(fixture, ['add', 'worker']));
  expectJsonResult(await squadCli(fixture, ['set', 'worker', `task=${task}`]));
  fs.appendFileSync(
    path.join(fixture.globalDir, 'ops.toml'),
    `\n[board]\nrefresh = "off"\n[bind]\nj = ${JSON.stringify(action)}\n`
  );
  await fixture.attachSessionClient('e2e');
  // Both pane processes are tracked by the fixture; joining avoids an unowned child.
  fixture.tmux(['join-pane', '-d', '-h', '-b', '-s', left.pane, '-t', board.pane]);
  fixture.tmux(['select-window', '-t', board.pane]);
  fixture.tmux(['select-pane', '-t', board.pane]);
  // Keep a 160-column board beside the working pane; notices must fit whole.
  fixture.tmux(['set-option', '-w', '-t', board.pane, 'window-size', 'manual']);
  fixture.tmux(['resize-window', '-t', board.pane, '-x', '321', '-y', '51']);
  fixture.tmux([
    'send-keys',
    '-t',
    board.pane,
    `tmt ops ui --squad product; printf '\\nHOST_BOARD_EXIT=%s\\n' "$?"`,
    'Enter',
  ]);
  await fixture.waitForCapture(
    (screen) => screen.includes('OPS / UI') && /2\s+Agents/.test(screen),
    board.pane
  );
  fixture.tmux(['send-keys', '-t', board.pane, '2']);
  await fixture.waitForCapture(
    (screen) => screen.includes('lead left-agent') && screen.includes('2 members'),
    board.pane
  );
  fixture.tmux(['send-keys', '-t', board.pane, 'Home', 'Right']);
  await selectHostWorker(fixture, board.pane);
  return { board: board.pane, left: left.pane, worker: worker.pane };
}

async function closeHostActionBoard(fixture: E2EFixture, board: string) {
  fixture.tmux(['send-keys', '-t', board, 'q']);
  await fixture.waitForCapture((screen) => /^HOST_BOARD_EXIT=0$/m.test(screen), board);
}

describe('squad on a private tmux server', { concurrent: false }, () => {
  it('dispatches authored j through real Core swaps and keeps the board focused on success and refusal', async () => {
    await withE2EFixture(async (fixture) => {
      const panes = await hostActionBoard(fixture, 'swap left', 'host action integration');
      const initial = hostPaneSlots(fixture);
      const caller = clientPlace(fixture);
      await fixture.waitForCapture((screen) => screen.includes('j swap left'), panes.board);
      const ids = Object.keys(initial).sort();
      fixture.tmux(['send-keys', '-t', panes.board, 'j']);
      await fixture.waitFor(
        () => hostPaneSlots(fixture)[panes.worker] === initial[panes.left],
        2_000,
        'worker exchanged into the left slot'
      );
      expect(hostPaneSlots(fixture)[panes.left]).toBe(initial[panes.worker]);
      expect(Object.keys(hostPaneSlots(fixture)).sort()).toEqual(ids);
      expect(hostPaneSlots(fixture)[panes.board]).toBe(initial[panes.board]);
      expect(clientPlace(fixture)).toBe(caller);
      await fixture.waitForCapture(
        (screen) => screen.includes('Swapped worker into the left pane beside the board.'),
        panes.board
      );
      // The same authored key still wins while the member conversation owns focus.
      fixture.tmux(['send-keys', '-t', panes.board, 'Tab']);
      await fixture.waitForCapture((screen) => screen.includes('Esc back'), panes.board);
      fixture.tmux(['send-keys', '-t', panes.board, 'j']);
      await fixture.waitForCapture(
        (screen) => screen.includes('worker is already beside the board.'),
        panes.board
      );
      expect(clientPlace(fixture)).toBe(caller);
      // Selecting the squad targets its lead and reverses the exact exchange.
      fixture.tmux(['send-keys', '-t', panes.board, 'Tab']);
      await fixture.waitForCapture((screen) => screen.includes('↑↓ select'), panes.board);
      fixture.tmux(['send-keys', '-t', panes.board, 'Escape']);
      await fixture.waitForCapture((screen) => /\/[ \t]+Search/.test(screen), panes.board);
      fixture.tmux(['send-keys', '-t', panes.board, 'Home']);
      await fixture.waitForCapture(
        (screen) => screen.includes('── product') && screen.includes('2 members'),
        panes.board
      );
      fixture.tmux(['send-keys', '-t', panes.board, 'j']);
      await fixture.waitFor(
        () => hostPaneSlots(fixture)[panes.left] === initial[panes.left],
        2_000,
        'lead exchanged back into the original left slot'
      );
      expect(hostPaneSlots(fixture)).toEqual(initial);
      expect(clientPlace(fixture)).toBe(caller);
      await fixture.waitForCapture(
        (screen) => screen.includes('Swapped left-agent into the left pane beside the board.'),
        panes.board
      );
      fixture.tmux(['send-keys', '-t', panes.board, 'j']);
      await fixture.waitForCapture(
        (screen) => screen.includes('left-agent is already beside the board.'),
        panes.board
      );
      expect(hostPaneSlots(fixture)).toEqual(initial);
      await selectHostWorker(fixture, panes.board);
      fixture.tmux(['kill-pane', '-t', panes.left]);
      const noNeighbour = hostPaneSlots(fixture);
      fixture.tmux(['send-keys', '-t', panes.board, 'j']);
      await fixture.waitForCapture(
        (screen) =>
          screen.includes('No visible neighbouring pane in that direction; nothing changed.'),
        panes.board
      );
      expect(hostPaneSlots(fixture)).toEqual(noNeighbour);
      expect(clientPlace(fixture)).toBe(caller);
      const replacement = fixture.createShellPane('host-replacement-left');
      fixture.tmux(['join-pane', '-d', '-h', '-b', '-s', replacement.pane, '-t', panes.board]);
      const workerId = String(durableIdentity(fixture, 'worker').id);
      fixture.tmux(['kill-pane', '-t', panes.worker]);
      const missingAgent = hostPaneSlots(fixture);
      fixture.tmux(['send-keys', '-t', panes.board, 'j']);
      // App names the frozen UUID in Core's refusal through its held roster.
      await fixture.waitForCapture(
        (screen) => screen.includes("Identity 'worker' was not found."),
        panes.board
      );
      expect(durableState(fixture).identities.find((row) => row.id === workerId)).toMatchObject({
        id: workerId,
        lifetime: 'temporary',
        retired_at_ms: expect.any(Number),
      });
      expect(hostPaneSlots(fixture)).toEqual(missingAgent);
      expect(clientPlace(fixture)).toBe(caller);
      await closeHostActionBoard(fixture, panes.board);
    });
  });

  it('dispatches a custom j command with verified board and member panes as single literal argv', async () => {
    await withE2EFixture(async (fixture) => {
      const script = path.join(fixture.workspace, 'host argument probe.mjs');
      const output = path.join(fixture.workspace, 'host argv.json');
      const forbidden = path.join(fixture.workspace, 'unexpected-shell-effect');
      fs.writeFileSync(
        script,
        `import {writeFileSync,renameSync} from 'node:fs';
const [output,...args]=process.argv.slice(2);
writeFileSync(output+'.tmp',JSON.stringify({pid:process.pid,args}));
renameSync(output+'.tmp',output);
`
      );
      const task = `one argument ; $(touch ${forbidden})`;
      // JSON quoting is for the Action/TOML grammars; no shell consumes this line.
      const command = `run ${[process.execPath, script, output].map((arg) => JSON.stringify(arg)).join(' ')} {board_pane} {pane} {task} {{left-of}}`;
      const panes = await hostActionBoard(fixture, command, task);
      const before = hostPaneSlots(fixture);
      const caller = clientPlace(fixture);
      expect(fs.existsSync(output)).toBe(false);
      await fixture.waitForCapture((screen) => screen.includes('j run'), panes.board);
      fixture.tmux(['send-keys', '-t', panes.board, 'j']);
      try {
        await fixture.waitFor(
          () => fs.existsSync(output),
          2_000,
          'custom command argv publication'
        );
      } catch (error) {
        throw new Error(`${String(error)}\n${fixture.capture(100, panes.board)}`, { cause: error });
      }
      const observed = JSON.parse(fs.readFileSync(output, 'utf8')) as {
        pid: number;
        args: string[];
      };
      expect(observed.args).toEqual([panes.board, panes.worker, task, '{left-of}']);
      expect(Number.isInteger(observed.pid) && observed.pid > 0).toBe(true);
      await fixture.waitFor(
        () => !fixture.mockProcessIsRunning(observed.pid),
        2_000,
        'custom command completion'
      );
      expect(fs.existsSync(forbidden)).toBe(false);
      expect(hostPaneSlots(fixture)).toEqual(before);
      expect(clientPlace(fixture)).toBe(caller);
      await closeHostActionBoard(fixture, panes.board);
    });
  });

  it('creates from the empty workspace only after exact review and keeps section keys out of fields', async () => {
    await withE2EFixture(async (fixture) => {
      installSquad(fixture);
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Ben']));
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'worker']));
      expectJsonResult(await squadCli(fixture, ['me', 'Ben']));
      const beforeRequests = requestAttempts(fixture);
      const beforeIdentities = durableState(fixture);
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('workspace-board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'tmt ops ui; echo WORKSPACE_BOARD_EXIT=$?',
        'Enter',
      ]);
      await fixture.waitForCapture((screen) => screen.includes('OPS / UI'), shell.pane);
      // Select Overview explicitly; startup-tab preferences do not own this scenario.
      fixture.tmux(['send-keys', '-t', shell.pane, '1']);
      await fixture.waitForCapture(
        (screen) =>
          screen.includes('OPS / UI') &&
          /^ AGENT[ \t]+SQUAD[ \t]+STATE[ \t]+TOK 1h[ \t]+MODEL\b/m.test(screen) &&
          /^   Ben[ \t]+Common pool[ \t]+\?[ \t]/m.test(screen) &&
          /^   worker[ \t]+Common pool[ \t]+\?[ \t]/m.test(screen) &&
          /Agents[ \t]+Squads[ \t]+\/[ \t]+Search/.test(screen),
        shell.pane
      );
      // Overview selects an agent; select the visible Common pool before expanding it.
      fixture.tmux(['send-keys', '-t', shell.pane, '2', 'Home', 'Right']);
      await fixture.waitForCapture((screen) => {
        const header = screen.split('\n').find((line) => line.includes('SQUAD / AGENT'));
        const rule = screen.split('\n').find((line) => line.startsWith('── Squads '));
        // The 160-cell frame gives the list 75 cells. Its section rule shares
        // the focus title with a one-cell divider; columns stay one row below.
        return (
          header !== undefined &&
          rule !== undefined &&
          /^ +SQUAD \/ AGENT[ \t]+STATE[ \t]+TOK 1h[ \t]+◷[ \t]+MODEL[ \t]*$/.test(
            header.slice(0, 75)
          ) &&
          rule.slice(76).startsWith('── Common pool ') &&
          /^[ \t]+├ Ben[ \t]+\?[ \t]/m.test(screen) &&
          /^[ \t]+└ worker[ \t]+\?[ \t]/m.test(screen)
        );
      }, shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'n']);
      await fixture.waitForCapture((screen) => screen.includes('Squad name'), shell.pane);
      fixture.tmux(['send-keys', '-l', '-t', shell.pane, 'product1']);
      // Digits are authored field input, never navigation to Overview/Agents.
      await fixture.waitForCapture(
        (screen) => screen.includes('product1') && screen.includes('Squad name'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture(
        (screen) => !screen.includes('Squad name') && screen.includes('OPS / UI'),
        shell.pane
      );
      const cancelled = await fixture.runJsonCli(['room', 'show', 'squad-product1']);
      expect(cancelled.code).not.toBe(0);
      expect(errorCode(cancelled)).toBe('ROOM_NOT_FOUND');
      expect(requestAttempts(fixture)).toEqual(beforeRequests);
      fixture.tmux(['send-keys', '-t', shell.pane, 'n']);
      await fixture.waitForCapture((screen) => screen.includes('Squad name'), shell.pane);
      fixture.tmux(['send-keys', '-l', '-t', shell.pane, 'product']);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitForCapture((screen) => screen.includes('Lead (optional'), shell.pane);
      fixture.tmux(['send-keys', '-l', '-t', shell.pane, 'Ben']);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitForCapture((screen) => screen.includes('Member (@agent'), shell.pane);
      fixture.tmux(['send-keys', '-l', '-t', shell.pane, 'worker']);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitForCapture(
        (screen) =>
          screen
            .split('\n')
            .some(
              (line) =>
                line.startsWith('Create squad · Member (@agent') &&
                line.includes('›') &&
                !line.includes('worker')
            ),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitForCapture(
        (screen) =>
          /Cancel[ \t]+Create squad/.test(screen) && screen.includes('New squad: product'),
        shell.pane
      );
      const beforeApply = await fixture.runJsonCli(['room', 'show', 'squad-product']);
      expect(beforeApply.code).not.toBe(0);
      expect(errorCode(beforeApply)).toBe('ROOM_NOT_FOUND');
      expect(requestAttempts(fixture)).toEqual(beforeRequests);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Tab', 'Tab', 'Enter']);
      await fixture.waitForCapture((screen) => screen.includes('Applied'), shell.pane);
      const room = expectJsonResult<{ room: { id: string; memberIds: string[] } }>(
        await fixture.runJsonCli(['room', 'show', 'squad-product'])
      ).room;
      expect(room.id).toMatch(/^[a-f0-9-]{36}$/);
      expect(room.memberIds).toHaveLength(2);
      expect(requestAttempts(fixture)).toEqual(beforeRequests);
      // Management changes membership; it does not retire or recreate identities.
      const afterCreate = durableState(fixture);
      expect(afterCreate.identities).toEqual(beforeIdentities.identities);
      expect(afterCreate.bindings).toEqual(beforeIdentities.bindings);
      expect(afterCreate.profiles).toEqual(beforeIdentities.profiles);
      expect(
        afterCreate.metadata.filter((row) => !String(row.key).startsWith('squad.product.'))
      ).toEqual(beforeIdentities.metadata);
      expect(afterCreate.metadata).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ key: 'squad.product.lead.marker', value: 'true' }),
          expect.objectContaining({ key: 'squad.product.state', value: 'working' }),
        ])
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture(
        (screen) => !/Cancel[ \t]+Create squad/.test(screen) && screen.includes('product'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, '2']);
      await fixture.waitForCapture((screen) => /2\s+Agents/.test(screen), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture(
        (screen) => screen.includes('WORKSPACE_BOARD_EXIT=0'),
        shell.pane
      );
    });
  });

  it.each(
    [80, 100, 160].flatMap((width) =>
      [
        { theme: 'tmt', noColor: false },
        { theme: 'tmt-light', noColor: false },
        { theme: 'tmt', noColor: true },
      ].map((look) => ({ width, ...look }))
    )
  )(
    'keeps a $width-column $theme UI (noColor=$noColor) clock-less until migration',
    async ({ width, theme, noColor }) => {
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
        fs.appendFileSync(config, `\n[board]\nrefresh = "off"\n[board.theme]\nbase = "${theme}"\n`);
        const beforeConfig = fs.readFileSync(config);
        const beforeJobs = storedDocument(path.join(ops, 'ops.db'), 'cron');
        expect(beforeJobs).toBeDefined();
        fs.renameSync(config, path.join(fixture.globalDir, 'squad.toml'));
        fs.renameSync(ops, path.join(fixture.globalDir, 'squad'));
        fs.unlinkSync(path.join(fixture.globalDir, '.ops-paths-v1'));
        fs.unlinkSync(path.join(fixture.globalDir, '.ops-paths-cutover-v1'));
        const lease = path.join(fixture.globalDir, 'squad', 'cron', 'clock.json');
        fs.mkdirSync(path.join(fixture.globalDir, 'squad', 'cron'), {
          recursive: true,
          mode: 0o700,
        });
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
        const heldLease = fs.readFileSync(lease);
        await fixture.attachSessionClient('e2e');
        const shell = fixture.createShellPane('pending-migration-board');
        fixture.tmux(['select-window', '-t', shell.pane]);
        fixture.tmux(['resize-window', '-t', shell.pane, '-x', String(width), '-y', '24']);
        // tmux changes the pane's tty size a moment after the resize returns. A board
        // started before that reads the old size, then clears and repaints the screen
        // when the new one arrives, which a capture can land in the middle of.
        const ttySize = fixture
          .tmux(['display-message', '-p', '-t', shell.pane, '#{pane_height} #{pane_width}'])
          .trim();
        fixture.tmux([
          'send-keys',
          '-t',
          shell.pane,
          `until [ "$(stty size)" = "${ttySize}" ]; do sleep 0.02; done; ` +
            `${noColor ? 'NO_COLOR=1 ' : ''}tmt ops ui --squad product; echo MIGRATION_BOARD_EXIT=$?`,
          'Enter',
        ]);
        await fixture.waitForCapture(
          (screen) =>
            screen.includes('OPS / UI') &&
            screen.includes('product') &&
            !screen.includes('Opening product') &&
            screen.includes('pane %41') &&
            screen.includes('Ops then retries'),
          shell.pane
        );
        fixture.tmux(['send-keys', '-t', shell.pane, 'Right', 'Down']);
        await fixture.waitForCapture(
          (screen) => screen.includes('worker') && screen.includes('Ops then retries'),
          shell.pane
        );
        const screen = fixture.capture(24, shell.pane);
        expect(screen).toContain(
          'Ops migration pending: quit the old board in pane %41 (q); Ops then retries.'
        );
        expect(
          Number(fixture.tmux(['display-message', '-p', '-t', shell.pane, '#{pane_width}']).trim())
        ).toBe(width);
        console.info(
          'OPS_MIGRATION_CAPTURE ' +
            JSON.stringify({
              width,
              theme,
              noColor,
              screen,
              ansi: fixture.tmux(['capture-pane', '-p', '-e', '-t', shell.pane]),
            })
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
        expect(pending.stderr).toContain('until that board exits');
        expect(pending.json!.jobs).toHaveLength(1);
        expect(pending.json!.jobs[0]).toMatchObject({
          id: added.job.id,
          message: added.job.message,
        });
        expect(fs.readFileSync(lease)).toEqual(heldLease);
        fs.unlinkSync(lease);
        // No CLI observation may perform the cutover: the same live UI must do it.
        await fixture.waitFor(
          () => fs.existsSync(path.join(fixture.globalDir, '.ops-paths-v1')),
          5_000,
          'UI-owned migration completion'
        );
        const nextLease = path.join(ops, 'cron', 'clock.json');
        await fixture.waitFor(() => fs.existsSync(nextLease), 5_000, 'Ops UI clock publication');
        expect(JSON.parse(fs.readFileSync(nextLease, 'utf8')).pid).not.toBe(123);
        await fixture.waitForCapture(
          (screen) =>
            screen.includes('worker') &&
            !screen.includes('Ops migration pending') &&
            !screen.includes('old clock PID 123') &&
            !screen.includes('legacy config/state stay active'),
          shell.pane
        );
        expect(fs.readFileSync(config)).toEqual(beforeConfig);
        expect(storedDocument(path.join(ops, 'ops.db'), 'cron')).toEqual(beforeJobs);
        // A compatible authored key proves refresh reads the promoted file.
        // Retired factory views no longer change the workspace shell.
        const adoptedConfig = Buffer.concat([beforeConfig, Buffer.from('\n[bind]\n0 = "help"\n')]);
        fs.writeFileSync(config, adoptedConfig);
        fixture.tmux(['send-keys', '-t', shell.pane, 'C-r', '?']);
        try {
          await fixture.waitFor(
            () => /\b0\s+show effective scoped bindings\b/.test(fixture.capture(24, shell.pane)),
            5_000,
            'manual refresh adopting the compatible key in effective Help'
          );
        } catch (error) {
          throw new Error(`${String(error)}\n${fixture.capture(24, shell.pane)}`, { cause: error });
        }
        fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
        await fixture.waitForCapture(
          (screen) => !screen.includes('effective scoped bindings') && /2\s+Agents/.test(screen),
          shell.pane
        );
        fixture.tmux(['send-keys', '-t', shell.pane, '0']);
        await fixture.waitForCapture(
          (screen) => screen.includes('effective scoped bindings'),
          shell.pane
        );
        fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
        await fixture.waitForCapture(
          (screen) => !screen.includes('effective scoped bindings') && /2\s+Agents/.test(screen),
          shell.pane
        );
        fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
        await fixture.waitForCapture(
          (screen) => screen.includes('MIGRATION_BOARD_EXIT=0'),
          shell.pane
        );
        expect(fs.existsSync(nextLease)).toBe(false);
        expect(fs.readFileSync(config)).toEqual(adoptedConfig);
        expect(storedDocument(path.join(ops, 'ops.db'), 'cron')).toEqual(beforeJobs);
        expect(fs.existsSync(path.join(fixture.globalDir, 'squad'))).toBe(false);
      });
    }
  );

  it('sends a scheduled slot once, refuses a second clock and releases its lease on shutdown', async () => {
    await withE2EFixture(
      async (fixture) => {
        installSquad(fixture);
        const home = path.join(fixture.root, 'clock-home');
        fs.mkdirSync(home);
        fixture.tmux(['set-environment', '-g', 'HOME', home]);
        fixture.tmux(['set-environment', '-g', 'XDG_CACHE_HOME', path.join(home, 'cache')]);
        fixture.tmux(['set-environment', '-g', 'TMT_HOME', fixture.globalDir]);
        expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Ben']));
        const peer = await fixture.createMockPane('clock-owner');
        expectJsonResult(await fixture.runJsonCli(['add', '--save', peer.pane, 'worker']));
        const owner = durableIdentity(fixture, 'worker').id;
        expectJsonResult(await squadCli(fixture, ['init', 'product', '--me', 'Ben']));
        expectJsonResult(await squadCli(fixture, ['add', 'worker']));
        const leasePath = path.join(fixture.globalDir, 'ops', 'cron', 'clock.json');
        const clock = await spawnRealTmuxCli(fixture, ['ops', 'squad', 'cron', 'run', '--json'], {
          name: 'primary-clock',
          json: false,
        });
        fs.writeFileSync(clock.releasePath, 'run');
        await fixture.waitFor(() => fs.existsSync(leasePath), 5_000, 'clock lease publication');
        const holder = JSON.parse(fs.readFileSync(leasePath, 'utf8'));
        expect(holder.pane).toBe(clock.pane);
        const second = await spawnRealTmuxCli(fixture, ['ops', 'squad', 'cron', 'run', '--json'], {
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
      fixture.tmux(['set-environment', '-g', 'TMT_HOME', fixture.globalDir]);
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
        const process = await spawnRealTmuxCli(
          fixture,
          ['ops', 'squad', 'cron', ...args, '--json'],
          {
            name: `cron-${commandNumber++}`,
            json: false,
          }
        );
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
        const db = new Database(path.join(fixture.globalDir, 'tmt.db'), { readonly: true });
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
      const db = new Database(path.join(fixture.globalDir, 'tmt.db'), { readonly: true });
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
      const taken = await opsCli(fixture, install);
      expect(errorCode(taken)).toBe('SQUAD_HOTKEY_TAKEN');
      expect(JSON.stringify(taken.json)).toContain('running server');
      expect(fs.readFileSync(conf, 'utf8')).toBe('set -g mouse on\n');

      fixture.tmux(['unbind-key', 'S']);
      expectJsonResult(await opsCli(fixture, install));
      expect(notedKeys(fixture)).toEqual(
        expect.arrayContaining(['S|tmt ops popup', 'B|tmt ops pane'])
      );
      expect(boundCommand(fixture, 'S')).toContain('ops ui --popup');
      expect(fs.readFileSync(conf, 'utf8')).toMatch(
        /^set -g mouse on\nsource-file -q '.*ops\.tmux\.conf' # tmt ops hotkeys\n$/
      );

      // The user rebinds B afterwards: removal must leave it alone.
      fixture.tmux(['bind-key', 'B', 'split-window']);
      const removed = expectJsonResult<{ unbound: string[] }>(
        await opsCli<{ unbound: string[] }>(fixture, ['hotkeys', 'remove', '--yes'])
      );
      expect(removed.unbound).toEqual(['S']);
      expect(boundCommand(fixture, 'S')).toBeUndefined();
      expect(boundCommand(fixture, 'B')).toBe('split-window');
    });
  });

  it('uses Workspace settings Actions for an empty room and persists explicit checklist updates without dispatch', async () => {
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
          'tmt ops ui --squad product; echo CHECKLIST_BOARD_EXIT=$?',
          'Enter',
        ]);
        await fixture.waitForCapture(
          (screen) =>
            /^[ \t]*[▸▾] product[ \t]+—[ \t]+0\b/m.test(screen) && screen.includes('0 members'),
          shell.pane
        );
        fixture.tmux(['send-keys', '-t', shell.pane, ',']);
        await fixture.waitForCapture(
          (screen) => screen.includes('Workspace settings') && screen.includes('Actions'),
          shell.pane
        );
        // Actions is the final entry; preceding preference choices may grow.
        fixture.tmux(['send-keys', '-t', shell.pane, ...Array(40).fill('Down'), 'Enter']);
        await fixture.waitForCapture(
          (screen) =>
            !screen.includes('Workspace settings') && screen.includes('Enter runs · Esc closes'),
          shell.pane
        );
        // Checklist can be below the viewport until the action menu scrolls to it.
        fixture.tmux(['send-keys', '-t', shell.pane, ...Array(40).fill('Down')]);
        await fixture.waitForCapture((screen) => /›\s*Checklist\b/.test(screen), shell.pane);
        fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
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
        const database = path.join(fixture.globalDir, 'ops', 'ops.db');
        const read = () => storedDocument(database, 'checklist', roomId);
        expect(read()).toBeUndefined();
        fixture.tmux(['send-keys', '-t', shell.pane, 'Down', 'Enter']);
        await fixture.waitFor(() => read() !== undefined, 5_000, 'checklist publication');
        let stored = JSON.parse(read()!.toString('utf8'));
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
          () => JSON.parse(read()!.toString('utf8')).items[0].completion === 'complete',
          5_000,
          'explicit completion'
        );
        stored = JSON.parse(read()!.toString('utf8'));
        expect(stored.items[0].revision).toBe(2);
        expect(stored.inventoryRevision).toBe(1);
        expect(durableState(fixture)).toEqual(state);
        expect(requestAttempts(fixture)).toEqual(beforeRequests);
        expect(fs.readFileSync(path.join(fixture.globalDir, 'ops.toml'))).toEqual(config);
        await fixture.waitForCapture((screen) => screen.includes('hidden by filters'), shell.pane);
        fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
        await fixture.waitForCapture(
          (screen) =>
            !screen.includes('Checklist') &&
            /^[ \t]*[▸▾] product[ \t]+—[ \t]+0\b/m.test(screen) &&
            screen.includes('0 members'),
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
      const listed = expectJsonResult<{
        identities: { name: string; pane: string | null; location?: { host: string } }[];
      }>(await fixture.runJsonCli(['ls']));
      const observed = listed.identities.find((row) => row.name === 'auth-fix');
      expect(observed?.pane).toBe(member);
      expect(observed?.location?.host).toBe('tmux');
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      const start = clientPlace(fixture);
      expect(start).toBe(`e2e/${shell.pane}`);

      // A named squad opens as the UUID-anchored target in Agents.
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'tmt ops ui --squad product --popup; echo BOARD_EXIT=$?',
        'Enter',
      ]);
      await fixture.waitForCapture(
        (screen) => screen.includes('product') && screen.includes('OPS / UI'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Right', 'Down']);
      await fixture.waitForCapture((screen) => screen.includes('auth-fix'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'j']);
      await fixture.waitForCapture((screen) => screen.includes('BOARD_EXIT=0'), shell.pane);
      expect(clientPlace(fixture)).toBe(`crew/${member}`);

      // The pane form: the same jump leaves the board running.
      fixture.tmux(['switch-client', '-t', shell.pane]);
      fixture.tmux(['send-keys', '-t', shell.pane, 'clear; tmt ops ui --squad product', 'Enter']);
      await fixture.waitForCapture(
        (screen) =>
          screen.includes('product') &&
          screen.includes('OPS / UI') &&
          !screen.includes('BOARD_EXIT'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Right', 'Down']);
      await fixture.waitForCapture((screen) => screen.includes('auth-fix'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'j']);
      await fixture.waitFor(() => clientPlace(fixture) === `crew/${member}`, 5_000, 'jumped');
      await new Promise((resolve) => setTimeout(resolve, 500));
      expect(fixture.capture(40, shell.pane)).toContain('auth-fix');
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture((screen) => !screen.includes('MEMBER'), shell.pane);
    });
  });
  it('reloads an open board in place when its installed binary is replaced or a reload is requested', async () => {
    await withE2EFixture(async (fixture) => {
      await squadWithMember(fixture);
      const docs = fixture
        .tmux(['new-window', '-d', '-t', 'crew', '-P', '-F', '#{pane_id}', 'cat'])
        .trim();
      expectJsonResult(await fixture.runJsonCli(['add', docs, 'docs-fix']));
      expectJsonResult(await squadCli(fixture, ['add', 'docs-fix']));

      // UI --squad is a session scope: these launches intentionally exclude Common pool.
      // A managed-looking install: PATH link -> current -> release. Each release records
      // its launch, then runs the built board under the PATH name, as the real one is run.
      const built = fs.realpathSync(path.join(fixture.wrapperDir, 'tmt-ops'));
      const install = path.join(fixture.workspace, 'reload-install');
      const cache = path.join(fixture.workspace, 'reload-cache');
      const launches = path.join(install, 'launches.log');
      for (const release of ['one', 'two']) {
        fs.mkdirSync(path.join(install, release), { recursive: true });
        const script = path.join(install, release, 'tmt-ops');
        fs.writeFileSync(
          script,
          `#!/bin/bash\necho ${release} >> '${launches}'\nexec -a "$0" '${built}' "$@"\n`
        );
        fs.chmodSync(script, 0o755);
      }
      const install_release = (release: string): void => {
        const staged = path.join(install, 'current.new');
        fs.rmSync(staged, { force: true });
        fs.symlinkSync(release, staged);
        fs.renameSync(staged, path.join(install, 'current'));
      };
      install_release('one');
      fs.rmSync(path.join(fixture.wrapperDir, 'tmt-ops'));
      fs.symlinkSync(
        path.join(install, 'current', 'tmt-ops'),
        path.join(fixture.wrapperDir, 'tmt-ops')
      );
      const launched = (): string =>
        fs.existsSync(launches)
          ? fs.readFileSync(launches, 'utf8').trim().split('\n').join(',')
          : '';
      const screen = (pane: string): string => fixture.capture(40, pane);
      // The selected-squad preview keeps its full roster; search filters the tree rows.
      const agentRows = (text: string): string =>
        text
          .split('\n')
          .filter((line) => /^[ \t]*[├└] /.test(line))
          .join('\n');

      const shell = fixture.createShellPane('board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        `export XDG_CACHE_HOME='${cache}'; tmt ops ui --squad product; echo BOARD_EXIT=$?`,
        'Enter',
      ]);
      // Registered cat panes are online, but have no provider activity observation.
      await fixture.waitForCapture(
        (screen) =>
          screen.includes('OPS / UI') &&
          screen.includes('1 squad · 2 agents') &&
          /^[ \t]*[▸▾] product[ \t]+\? \?[ \t]/m.test(screen) &&
          screen.includes('2 members'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Right']);
      await fixture.waitFor(
        () => {
          const rows = agentRows(screen(shell.pane));
          return rows.includes('auth-fix') && rows.includes('docs-fix');
        },
        10_000,
        'both members on the board'
      );
      // A filter is the place the reload must bring back.
      fixture.tmux(['send-keys', '-t', shell.pane, '/', 'docs', 'Enter']);
      await fixture.waitFor(
        () => {
          const rows = agentRows(screen(shell.pane));
          return rows.includes('docs-fix') && !rows.includes('auth-fix');
        },
        10_000,
        'filtered board'
      );
      expect(launched()).toBe('one');

      // The installed binary is replaced: the idle board restarts itself in its pane.
      install_release('two');
      await fixture.waitFor(() => launched() === 'one,two', 15_000, 'second release launched');
      await fixture.waitFor(
        () => {
          const now = screen(shell.pane);
          const rows = agentRows(now);
          return (
            now.includes('Board reloaded · tmt-ops') &&
            rows.includes('docs-fix') &&
            !rows.includes('auth-fix')
          );
        },
        15_000,
        'the reloaded board keeps its filter'
      );
      expect(screen(shell.pane)).not.toContain('BOARD_EXIT');

      // A search being typed defers the reload; finishing it lets the reload through.
      fixture.tmux(['send-keys', '-t', shell.pane, '/']);
      install_release('one');
      await new Promise((resolve) => setTimeout(resolve, 3_500));
      expect(launched()).toBe('one,two');
      expect(screen(shell.pane)).toContain('reload waiting');
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitFor(() => launched() === 'one,two,one', 15_000, 'reload after the search');

      // One command asks every open board; boards started after it are unaffected.
      await fixture.waitFor(
        () => screen(shell.pane).includes('Board reloaded · tmt-ops'),
        15_000,
        'board back after the replacement'
      );
      const asker = fixture.createShellPane('ask');
      fixture.tmux([
        'send-keys',
        '-t',
        asker.pane,
        `XDG_CACHE_HOME='${cache}' tmt ops ui --reload-all; echo ASK_EXIT=$?`,
        'Enter',
      ]);
      await fixture.waitForCapture((text) => text.includes('ASK_EXIT=0'), asker.pane);
      await fixture.waitFor(() => launched() === 'one,two,one,one', 15_000, 'reload on request');
      await fixture.waitFor(
        () => {
          const rows = agentRows(screen(shell.pane));
          return rows.includes('docs-fix') && !rows.includes('auth-fix');
        },
        15_000,
        'the filter survives every reload'
      );
      expect(screen(shell.pane)).toContain('Scope: product');
      // Escape first clears the retained search, then the argv scope. The
      // scope is ephemeral session state, independent of the reapplied filter.
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitFor(
        () => {
          const now = screen(shell.pane);
          const rows = agentRows(now);
          return (
            now.includes('Scope: product') && rows.includes('docs-fix') && rows.includes('auth-fix')
          );
        },
        5_000,
        'search clears before scope'
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitFor(
        () => !screen(shell.pane).includes('Scope:') && screen(shell.pane).includes('Common pool'),
        5_000,
        'explicit scope clear'
      );
      fixture.tmux(['send-keys', '-t', shell.pane, '/', 'docs', 'Enter']);
      await fixture.waitFor(
        () => {
          const now = screen(shell.pane);
          const rows = agentRows(now);
          return !now.includes('Scope:') && rows.includes('docs-fix') && !rows.includes('auth-fix');
        },
        5_000,
        'independent filter reapplied after scope clear'
      );
      fixture.tmux([
        'send-keys',
        '-t',
        asker.pane,
        `XDG_CACHE_HOME='${cache}' tmt ops ui --reload-all; echo CLEAR_RELOAD_EXIT=$?`,
        'Enter',
      ]);
      await fixture.waitForCapture((text) => text.includes('CLEAR_RELOAD_EXIT=0'), asker.pane);
      await fixture.waitFor(
        () => launched() === 'one,two,one,one,one',
        15_000,
        'reload after scope clear'
      );
      await fixture.waitFor(
        () => {
          const now = screen(shell.pane);
          const rows = agentRows(now);
          return (
            now.includes('Board reloaded · tmt-ops') &&
            !now.includes('Scope:') &&
            rows.includes('docs-fix') &&
            !rows.includes('auth-fix')
          );
        },
        15_000,
        'cleared scope and the independent search survive reload'
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture((text) => text.includes('BOARD_EXIT=0'), shell.pane);
    });
  }, 120_000);
});
