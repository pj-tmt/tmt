import fs from 'node:fs';
import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { assertPrivateSocket } from '../support/private-tmux.js';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import Database from 'better-sqlite3';
import { spawnRealTmuxCli, releaseRealTmuxCli, readRealTmuxCli } from './real-tmux-caller.js';
import { describe, expect, it } from 'vite-plus/test';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import { expectJsonResult } from './cli-assertions.js';
import { durableIdentity, durableState } from './identity-state-oracle.js';
import { requestAttempts, requestResponses } from './request-state-oracle.js';
import { publishDigestStep } from './claude-digest-step.js';
import { readCompleteJsonLines } from './harness.js';
import { waitForFileContent } from './wait-for-file.js';
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

async function squadWithMember(fixture: E2EFixture, command = 'cat'): Promise<string> {
  installSquad(fixture);
  const member = fixture
    .tmux(['new-session', '-d', '-s', 'crew', '-P', '-F', '#{pane_id}', command])
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
  await fixture.waitForCapture((screen) => /\/[ \t]+worker[ \t]+1 of 2[ \t]+×/.test(screen), board);
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
    (screen) => screen.includes('OPS / UI') && /1\s+Agents/.test(screen),
    board.pane
  );
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

  it('creates one saved member through the public host template and joins the frozen squad', async () => {
    await withE2EFixture(async (fixture) => {
      installSquad(fixture);
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Ben']));
      expectJsonResult(await squadCli(fixture, ['init', 'product', '--me', 'Ben']));
      const before = durableState(fixture);
      const socket = assertPrivateSocket(fixture.socketPath, fixture.socketRoot);
      const shellWord = (word: string) => `'${word.replaceAll("'", "'\\''")}'`;
      // The published script invokes tmux by name. Only this board inherits
      // the scenario shim; every invocation is pinned to its asserted -S.
      const shim = path.join(fixture.root, 'launch-bin');
      fs.mkdirSync(shim);
      writeExecutable(
        path.join(shim, 'tmux'),
        `#!/bin/sh\nunset TMUX TMUX_PANE\nexec ${shellWord(fixture.tmuxPath)} -S ${shellWord(socket)} "$@"\n`
      );
      const log = path.join(fixture.root, 'new-member-input');
      writeExecutable(
        path.join(shim, 'ops-new-peer'),
        `#!/bin/sh\nprintf 'NEW_MEMBER_READY\\n'\nwhile IFS= read -r line; do printf '%s\\n' "$line" >> ${shellWord(log)}; printf 'NEW_MEMBER_REPLY:%s\\n' "$line"; done\n`
      );
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('new-member-board');
      fixture.tmux(['resize-window', '-t', shell.pane, '-x', '150', '-y', '43']);
      fixture.tmux(['select-window', '-t', shell.pane]);
      fixture.tmux([
        'send-keys',
        '-l',
        '-t',
        shell.pane,
        `PATH=${shellWord(shim)}:$PATH tmt ops ui; echo NEW_MEMBER_BOARD_EXIT=$?`,
      ]);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitForCapture((screen) => /\/[ \t]+Search/.test(screen), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Home', 'Down', 'n']);
      await fixture.waitForCapture(
        (screen) => screen.includes('New member') && screen.includes('agent name'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-l', '-t', shell.pane, 'new-worker']);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Tab']);
      fixture.tmux(['send-keys', '-l', '-t', shell.pane, path.join(shim, 'ops-new-peer')]);
      // Model remains provider-default, Squad remains product, Host/session
      // remain tmux/product, and Saved remains selected.
      fixture.tmux(['send-keys', '-t', shell.pane, ...Array(7).fill('Tab')]);
      await fixture.waitForCapture(
        (screen) =>
          screen.includes('Will run') &&
          screen.includes('new-worker') &&
          screen.includes('ops-new-peer'),
        shell.pane
      );
      expect(durableState(fixture)).toEqual(before);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitForCapture(
        (screen) => screen.includes('New member') && screen.includes('Opens with'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'End']);
      await fixture.waitForCapture((screen) => {
        const position = screen.match(/Read · \d+–(\d+) of (\d+)/);
        return !position || position[1] === position[2];
      }, shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Tab', 'Enter']);
      await fixture.waitForCapture(
        (screen) => screen.includes('Created new-worker in product.'),
        shell.pane
      );
      const created = durableIdentity(fixture, 'new-worker');
      expect(created.lifetime).toBe('saved');
      expect(before.identities.some((row) => row.id === created.id)).toBe(false);
      expect(
        durableState(fixture).identities.filter((row) => row.name === 'new-worker')
      ).toHaveLength(1);
      const listing = expectJsonResult(await fixture.runJsonCli(['ls'])) as {
        identities: Array<{
          id: string;
          pane?: string;
          target?: string;
          location?: { host: string; session?: string };
        }>;
      };
      const row = listing.identities.find((entry) => entry.id === created.id)!;
      expect(row.location).toMatchObject({ host: 'tmux', session: 'product' });
      const member = fixture
        .tmux(['list-panes', '-a', '-F', '#{pane_id}|#{session_name}'])
        .trim()
        .split('\n')
        .find((line) => line.endsWith('|product'))!
        .split('|')[0]!;
      expect(fixture.tmux(['show-options', '-p', '-v', '-t', member, '@tmt.opened']).trim()).toBe(
        '1'
      );
      await fixture.waitForCapture((screen) => screen.includes('NEW_MEMBER_READY'), member);
      fixture.tmux(['send-keys', '-l', '-t', member, 'causal-new-member-4582']);
      fixture.tmux(['send-keys', '-t', member, 'Enter']);
      await fixture.waitForCapture(
        (screen) => screen.includes('NEW_MEMBER_REPLY:causal-new-member-4582'),
        member
      );
      expect(fs.readFileSync(log, 'utf8')).toBe('causal-new-member-4582\n');
      const squad = expectJsonResult<{
        sections: Array<{ rows: Array<{ id: string }> }>;
      }>(await squadCli(fixture, ['ls', '--squad', 'product']));
      expect(
        squad.sections.flatMap((section) => section.rows).some((entry) => entry.id === created.id)
      ).toBe(true);
      // Observe the modal closing before q; an adjacent Escape+q can be Alt-q.
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture(
        (screen) => !screen.includes('New member') && screen.includes('new-worker'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture(
        (screen) => screen.includes('NEW_MEMBER_BOARD_EXIT=0'),
        shell.pane
      );
    });
  });

  it('reaches the squad settings from the workspace menu, flips the idle alert with Space and returns to the menu on Esc', async () => {
    await withE2EFixture(async (fixture) => {
      await squadWithMember(fixture);
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('idle-alert-board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      fixture.tmux(['send-keys', '-t', shell.pane, 'tmt ops ui; echo IDLE_BOARD_EXIT=$?', 'Enter']);
      await fixture.waitForCapture((screen) => /\/[ \t]+Search/.test(screen), shell.pane);
      // Agents tab opens on the Squads tree: first row (Common pool), then the squad row below it.
      fixture.tmux(['send-keys', '-t', shell.pane, 'Home', 'Down']);
      await fixture.waitForCapture((screen) => screen.includes('SQUAD / AGENT'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, ',']);
      await fixture.waitForCapture(
        (screen) => screen.includes('Squad settings: product'),
        shell.pane
      );
      // The entry is last; Down saturates there and never lands on the rule above it.
      fixture.tmux(['send-keys', '-t', shell.pane, ...Array(40).fill('Down'), 'Enter']);
      // The overlay opens with the idle alert switch selected, so Space is offered at once.
      await fixture.waitForCapture((screen) => screen.includes('Space on/off'), shell.pane);
      const config = path.join(fixture.globalDir, 'ops.toml');
      fixture.tmux(['send-keys', '-t', shell.pane, 'Space']);
      await fixture.waitFor(
        () =>
          fs.existsSync(config) &&
          /idle_alert[\s\S]*enabled\s*=\s*true/.test(fs.readFileSync(config, 'utf8')),
        10_000,
        'idle alert saved through the board'
      );
      const read = await squadCli<{ entries: { key: string; value: unknown }[] }>(fixture, [
        'config',
        'show',
        '--squad',
        'product',
      ]);
      expect(read.code, read.stderr || read.stdout).toBe(0);
      expect(read.json!.entries.find((entry) => entry.key === 'idle_alert.enabled')?.value).toBe(
        true
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture(
        (screen) =>
          screen.includes('Squad settings: product') && screen.includes('Workspace settings'),
        shell.pane
      );
      // Leave the board before the fixture removes the state it still writes.
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture((screen) => !screen.includes('Workspace settings'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture((screen) => screen.includes('IDLE_BOARD_EXIT=0'), shell.pane);
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
      // Select the flat Agents list explicitly; startup-tab preferences do not own this scenario.
      fixture.tmux(['send-keys', '-t', shell.pane, 'f']);
      await fixture.waitForCapture(
        (screen) =>
          screen.includes('OPS / UI') &&
          /^ AGENT[ \t]+SQUAD[ \t]+STATE[ \t]+TOK 1h[ \t]+MODEL\b/m.test(screen) &&
          /^   Ben[ \t]+Common pool[ \t]+\?[ \t]/m.test(screen) &&
          /^   worker[ \t]+Common pool[ \t]+\?[ \t]/m.test(screen) &&
          /f[ \t]+Squads[ \t]+Agents[ \t]+Pinned[ \t─━]+\/[ \t]+Search/.test(screen),
        shell.pane
      );
      // The flat list selects an agent; return to the Squads tree (f cycles through Pinned),
      // then select the visible Common pool before expanding it.
      fixture.tmux(['send-keys', '-t', shell.pane, 'f', 'f', 'Home', 'Right']);
      await fixture.waitForCapture((screen) => {
        const header = screen.split('\n').find((line) => line.includes('SQUAD / AGENT'));
        const rule = screen.split('\n').find((line) => line.includes('f  Squads '));
        // The 160-cell frame gives the list 75 cells. Its controls rule shares
        // the Talk/Measure title with a one-cell divider; columns stay one row below.
        return (
          header !== undefined &&
          rule !== undefined &&
          /^ +SQUAD \/ AGENT( [↑↓])?[ \t]+STATE[ \t]+TOK 1h[ \t]+◷[ \t]+MODEL \/ ALERT[ \t]*$/.test(
            header.slice(0, 75)
          ) &&
          /Talk[ \t]+Measure[ \t]+[─━]{2} Common pool /.test(rule.slice(75)) &&
          /^[ \t]+├ Ben[ \t]+\?[ \t]/m.test(screen) &&
          /^[ \t]+└ worker[ \t]+\?[ \t]/m.test(screen)
        );
      }, shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'N']);
      await fixture.waitForCapture((screen) => screen.includes('Squad name'), shell.pane);
      fixture.tmux(['send-keys', '-l', '-t', shell.pane, 'product1']);
      // Digits are authored field input, never navigation to the Agents or Digest tab.
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
      fixture.tmux(['send-keys', '-t', shell.pane, 'N']);
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
      fixture.tmux(['send-keys', '-t', shell.pane, '1']);
      await fixture.waitForCapture((screen) => /1\s+Agents/.test(screen), shell.pane);
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
          (screen) => !screen.includes('effective scoped bindings') && /1\s+Agents/.test(screen),
          shell.pane
        );
        fixture.tmux(['send-keys', '-t', shell.pane, '0']);
        await fixture.waitForCapture(
          (screen) => screen.includes('effective scoped bindings'),
          shell.pane
        );
        fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
        await fixture.waitForCapture(
          (screen) => !screen.includes('effective scoped bindings') && /1\s+Agents/.test(screen),
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
      // ops.toml may not exist: the user lives in Core, not in this file.
      const configPath = path.join(fixture.globalDir, 'ops.toml');
      const readConfig = () => (fs.existsSync(configPath) ? fs.readFileSync(configPath) : null);
      const config = readConfig();
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
        // Down saturates on the final entry (the selected squad's settings link); Up steps over the
        // rule and Panels to Actions, which precedes them. Preceding preference choices may grow.
        fixture.tmux([
          'send-keys',
          '-t',
          shell.pane,
          ...Array(40).fill('Down'),
          'Up',
          'Up',
          'Enter',
        ]);
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
        expect(readConfig()).toEqual(config);
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

  it('publishes Notes completion to its chip, member row and global todo count', async () => {
    await withE2EFixture(async (fixture) => {
      await squadWithMember(fixture);
      const shown = expectJsonResult(
        await fixture.runJsonCli<{ room: { id: string } }>(['room', 'show', 'squad-product'])
      );
      const item = randomUUID();
      const created = expectJsonResult(
        await squadCli<{ current: { member: { id: string } } }>(fixture, [
          'checklist',
          'create',
          '--room',
          shown.room.id,
          '--member',
          'auth-fix',
          '--item',
          item,
          '--expect-inventory',
          'absent',
          'Finish the causal Notes check',
        ])
      );
      const database = path.join(fixture.globalDir, 'ops', 'ops.db');
      const read = () =>
        JSON.parse(
          storedDocument(
            database,
            'checklist',
            `${shown.room.id}/${created.current.member.id}`
          )!.toString('utf8')
        );
      expect(read().items[0]).toMatchObject({ id: item, completion: 'open', revision: 1 });
      const beforeRequests = requestAttempts(fixture);
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('notes-count-board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'tmt ops ui --squad product; echo NOTES_COUNT_BOARD_EXIT=$?',
        'Enter',
      ]);
      const memberRow = (screen: string) =>
        screen
          .split('\n')
          .find((line) => /^[ \t]*[├└][^│\n]*auth-fix\b/.test(line))
          ?.split('│')[0];
      await fixture.waitForCapture((screen) => screen.includes('OPS / UI'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, '1']);
      await fixture.waitForCapture((screen) => /\/[ \t]+Search/.test(screen), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, '/', 'auth-fix', 'Enter']);
      await fixture.waitForCapture(
        (screen) => /\/[ \t]+auth-fix[ \t]+1 of 1[ \t]+×/.test(screen),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'End']);
      await fixture.waitForCapture(
        (screen) =>
          screen.includes('○ 1 todo') &&
          memberRow(screen)?.includes('· 1 todo') === true &&
          screen.includes('1 todo · 0 done'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'N']);
      await fixture.waitForCapture(
        (screen) => screen.includes('Finish the causal Notes check'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitFor(
        () => read().items[0].completion === 'complete',
        5_000,
        'Notes Done publication'
      );
      expect(read().items[0]).toMatchObject({ id: item, completion: 'complete', revision: 2 });
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture((screen) => {
        const row = memberRow(screen);
        return (
          row !== undefined &&
          !row.includes('todo') &&
          screen.includes('○ 0 todo') &&
          screen.includes('0 todo · 1 done')
        );
      }, shell.pane);
      expect(requestAttempts(fixture)).toEqual(beforeRequests);
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture(
        (screen) => screen.includes('NOTES_COUNT_BOARD_EXIT=0'),
        shell.pane
      );
    });
  });

  it('asks the terminal for the mouse again after a resize or a closed overlay, so the wheel stays with the board', async () => {
    await withE2EFixture(async (fixture) => {
      installSquad(fixture);
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Ben']));
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('wheel-board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'tmt ops ui; echo WHEEL_BOARD_EXIT=$?',
        'Enter',
      ]);
      await fixture.waitForCapture((screen) => screen.includes('OPS / UI'), shell.pane);

      // tmux sends the wheel to the pane only while the pane asks for mouse reports
      // (`mouse_any_flag`); without them its default binding enters copy-mode.
      const reports = () =>
        fixture
          .tmux([
            'display-message',
            '-p',
            '-t',
            shell.pane,
            '#{mouse_any_flag}#{mouse_sgr_flag}#{pane_in_mode}',
          ])
          .trim();
      const dropRequest = () => {
        // What a nested client, a multiplexer or an emulator reset does to the request.
        const tty = fixture.tmux(['display-message', '-p', '-t', shell.pane, '#{pane_tty}']).trim();
        fs.writeFileSync(tty, '\u001b[?1000l\u001b[?1002l\u001b[?1003l\u001b[?1015l\u001b[?1006l');
      };
      await fixture.waitFor(() => reports() === '110', 5_000, 'board asked for mouse reports');

      dropRequest();
      await fixture.waitFor(() => reports() === '000', 2_000, 'request dropped');
      fixture.tmux(['resize-window', '-t', shell.pane, '-x', '90', '-y', '24']);
      await fixture.waitFor(() => reports() === '110', 5_000, 'mouse asked again after resize');

      dropRequest();
      await fixture.waitFor(() => reports() === '000', 2_000, 'request dropped again');
      fixture.tmux(['send-keys', '-t', shell.pane, '?']);
      await fixture.waitForCapture((screen) => screen.includes('help'), shell.pane);
      expect(reports(), 'an open overlay does not ask').toBe('000');
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitFor(() => reports() === '110', 5_000, 'mouse asked again after overlay');

      // An idle board has no resize, focus or overlay to repair the request: the
      // periodic refresh (5 s by default) asks again with no user action.
      dropRequest();
      await fixture.waitFor(() => reports() === '000', 2_000, 'request dropped on an idle board');
      await fixture.waitFor(() => reports() === '110', 10_000, 'mouse asked again by the refresh');

      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture((screen) => screen.includes('WHEEL_BOARD_EXIT=0'), shell.pane);
    });
  });

  it('pairs a raw Ops chat with its captured turn and settles only the oldest question', async () => {
    await withE2EFixture(async (fixture) => {
      installSquad(fixture);
      const home = path.join(fixture.root, 'passthrough-home');
      const sessionId = randomUUID();
      const tree = path.join(home, '.claude', 'projects', '-workspace');
      fs.mkdirSync(tree, { recursive: true });
      const transcript = path.join(tree, `${sessionId}.jsonl`);
      fs.writeFileSync(transcript, '');
      const cliEnv: NodeJS.ProcessEnv = {
        ...process.env,
        HOME: home,
        TMT_HOME: fixture.globalDir,
        PATH: `${fixture.wrapperDir}:${path.dirname(fixture.executables.cli.executable)}:${process.env.PATH ?? ''}`,
        XDG_CACHE_HOME: path.join(home, 'cache'),
        TMUX_TMPDIR: fixture.socketRoot,
      };
      delete cliEnv.TMUX;
      delete cliEnv.TMUX_PANE;
      const cli = (args: string[], input?: unknown) =>
        JSON.parse(
          execFileSync(
            fixture.executables.cli.executable,
            [...fixture.executables.cli.args, ...args],
            {
              env: cliEnv,
              input: input === undefined ? undefined : JSON.stringify(input),
              encoding: 'utf8',
              timeout: 10_000,
            }
          )
        );
      const api = (operation: string, input: unknown, identity?: string) =>
        cli(['api'], { version: 1, operation, input, ...(identity ? { identity } : {}) });
      cli(['setup', 'claude', '--usage', '--yes', '--json']);
      fs.copyFileSync('/opt/tmt-tests/claude', path.join(fixture.wrapperDir, 'claude'));
      fs.chmodSync(path.join(fixture.wrapperDir, 'claude'), 0o755);
      const worker = fixture.createShellPane('passthrough-worker');
      const log = path.join(fixture.root, 'passthrough-worker.log');
      const status = path.join(fixture.root, 'passthrough-worker.status');
      const mock = fileURLToPath(new URL('./mock-claude-channel.mjs', import.meta.url));
      const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
      const command = [
        'env',
        `HOME=${home}`,
        `MOCK_CHANNEL_LOG=${log}`,
        `MOCK_DB=${path.join(fixture.globalDir, 'tmt.db')}`,
        'MOCK_AUTOREPLY=0',
        `MOCK_SESSION_ID=${sessionId}`,
        'MOCK_LAUNCH_HOOKS=1',
        `MOCK_TURN_TRANSCRIPT=${transcript}`,
        `TMT_TEST_CLAUDE_MOCK=${mock}`,
        `TMT_TEST_CLAUDE_NODE=${process.execPath}`,
        'TMT_TEST_CLAUDE_NATIVE_CHANNEL=1',
        fixture.executables.cli.executable,
        ...fixture.executables.cli.args,
        'run',
        '-s',
        'chat-worker',
        path.join(fixture.wrapperDir, 'claude'),
      ]
        .map(quote)
        .join(' ');
      fixture.tmux([
        'send-keys',
        '-t',
        worker.pane,
        '-l',
        `${command}; printf '%s' "$?" > ${quote(status)}`,
      ]);
      fixture.tmux(['send-keys', '-t', worker.pane, 'Enter']);
      const events = () => readCompleteJsonLines<{ event: string; line?: string }>(log);
      const event = async (name: string) => {
        await fixture.waitFor(
          () => events().some((item) => item.event === name || item.event === 'hook-error'),
          15_000,
          name
        );
        expect(events().filter((item) => item.event === 'hook-error')).toEqual([]);
        expect(events().some((item) => item.event === name)).toBe(true);
      };
      const step = async (name: string, fields: Record<string, unknown>) => {
        await publishDigestStep(log, { event: name, transcript, ...fields });
        await event(name);
      };
      try {
        await event('prompt-recorded');
        await step('initial-stop', { message: 'An earlier unrelated turn' });
        const user = cli(['identity', 'create', 'Ben', '--json']).identity.id as string;
        cli(['identity', 'meta', 'set', 'user', 'true', '--identity', user, '--json']);
        expectJsonResult(await squadCli(fixture, ['init', 'product', '--me', 'Ben']));
        expectJsonResult(await squadCli(fixture, ['add', 'chat-worker']));
        const member = durableIdentity(fixture, 'chat-worker')!;
        const recipient = member.id;
        expect(
          api('identities.status', { identityIds: [recipient] }).identities[0].canPassthrough
        ).toEqual({ available: true });
        const questions = ['First question?', 'Second question?'].map(
          (message) =>
            api(
              'dispatch.create',
              {
                operationId: randomUUID(),
                recipientIds: [user],
                kind: 'request',
                message,
              },
              recipient
            ).items[0].requestId as string
        );
        await fixture.attachSessionClient('e2e');
        const board = fixture.createShellPane('passthrough-board');
        fixture.tmux(['select-window', '-t', board.pane]);
        fixture.tmux([
          'send-keys',
          '-t',
          board.pane,
          'tmt ops ui --squad product; echo PASSTHROUGH_BOARD_EXIT=$?',
          'Enter',
        ]);
        await fixture.waitForCapture((screen) => screen.includes('OPS / UI'), board.pane);
        fixture.tmux(['send-keys', '-t', board.pane, '1']);
        await fixture.waitForCapture((screen) => /\/[ \t]+Search/.test(screen), board.pane);
        fixture.tmux(['send-keys', '-t', board.pane, '/', 'chat-worker', 'Enter', 'End']);
        await fixture.waitForCapture(
          (screen) => screen.includes('⏎ to message chat-worker'),
          board.pane
        );
        fixture.tmux(['send-keys', '-t', board.pane, 'Enter']);
        fixture.tmux(['send-keys', '-t', board.pane, '-l', '/yes!']);
        fixture.tmux(['send-keys', '-t', board.pane, 'Enter']);
        await fixture.waitFor(
          () => events().some((item) => item.event === 'paste' && item.line === '/yes！'),
          15_000,
          'raw chat reached the mock'
        );
        const sends = () =>
          requestAttempts(fixture).filter(
            (row) => row.originator_identity_id === user && row.recipient_identity_id === recipient
          );
        await fixture.waitFor(
          () =>
            sends().length === 1 &&
            requestResponses(fixture).some((row) => row.request_id === questions[0]),
          10_000,
          'one send and oldest settlement'
        );
        expect(sends()[0]!.message_text).toBe('/yes!');
        expect(
          requestResponses(fixture).filter((row) => questions.includes(row.request_id))
        ).toEqual([expect.objectContaining({ request_id: questions[0], body: '/yes!' })]);
        const request = sends()[0]!.request_id;
        await step('chat-prompt', { prompt: true, promptText: '/yes！' });
        await fixture.waitFor(
          () => api('requests.show', { requestId: request }).messageState?.state === 'working',
          5_000,
          'Core observes the bound turn working'
        );
        const workingText = api('requests.show', { requestId: request }).messageState.reason.text;
        await fixture.waitForCapture((screen) => screen.includes(workingText), board.pane);
        await step('chat-stop', { message: 'Captured answer from this turn' });
        await fixture.waitFor(
          () => api('requests.show', { requestId: request }).passthrough?.state === 'answered',
          10_000,
          'captured request answer'
        );
        expect(api('requests.show', { requestId: request }).passthrough.answer.text).toBe(
          'Captured answer from this turn'
        );
        await fixture.waitForCapture(
          (screen) =>
            screen.includes('turn output') && screen.includes('Captured answer from this turn'),
          board.pane
        );
        expect(sends()).toHaveLength(1);
        fixture.tmux(['send-keys', '-t', board.pane, 'Escape', 'Escape', 'q']);
        await fixture.waitForCapture(
          (screen) => screen.includes('PASSTHROUGH_BOARD_EXIT=0'),
          board.pane
        );
      } finally {
        fs.writeFileSync(`${log}.quit`, '');
        expect(
          await waitForFileContent(status, { description: 'passthrough mock foreground exited' })
        ).toBe('0');
      }
    });
  }, 60_000);

  it('keeps a dragged chat selection and its Quote button while the sent line ticks', async () => {
    await withE2EFixture(async (fixture) => {
      await squadWithMember(fixture);
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('quote-board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'tmt ops ui --squad product; echo QUOTE_BOARD_EXIT=$?',
        'Enter',
      ]);
      await fixture.waitForCapture((screen) => screen.includes('OPS / UI'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, '1']);
      await fixture.waitForCapture((screen) => /\/[ \t]+Search/.test(screen), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, '/', 'auth-fix', 'Enter', 'End']);
      await fixture.waitForCapture(
        (screen) => screen.includes('⏎ to message auth-fix'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      fixture.tmux(['send-keys', '-t', shell.pane, '-l', 'hello there']);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitForCapture((screen) => screen.includes('auth-fix: '), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);

      const screen = () => fixture.tmux(['capture-pane', '-p', '-t', shell.pane]);
      const where = (needle: string) => {
        const rows = screen().split('\n');
        const y = rows.findIndex((row) => row.includes(needle));
        let evidence = '';
        if (y < 0) {
          const attempts = requestAttempts(fixture);
          evidence = `\nStored attempts: ${JSON.stringify(attempts)}\nAfter store read:\n${screen()}`;
        }
        expect(y, `${needle} on screen:\n${rows.join('\n')}${evidence}`).toBeGreaterThanOrEqual(0);
        return { x: [...rows[y]!.slice(0, rows[y]!.indexOf(needle))].length, y };
      };
      // Pointer reports as the terminal sends them: press, motion with the button held, release.
      const pointer = (code: number, final: 'M' | 'm', x: number, y: number) =>
        fixture.tmux([
          'send-keys',
          '-t',
          shell.pane,
          '-l',
          `\u001b[<${code};${x + 1};${y + 1}${final}`,
        ]);
      const age = () => /Ben → auth-fix\s+(\d+)s ago/.exec(screen())?.[1];

      const text = where('hello there');
      pointer(0, 'M', text.x, text.y);
      pointer(32, 'M', text.x + 'hello there'.length, text.y);
      pointer(0, 'm', text.x + 'hello there'.length, text.y);
      await fixture.waitForCapture((current) => current.includes('q› Quote'), shell.pane);

      // The exchange stamp repaints the pane while the selection stands.
      const seen = age();
      expect(seen, 'the sent exchange shows its age').toBeDefined();
      await fixture.waitFor(() => age() !== seen, 15_000, 'the age line ticked');
      expect(screen(), 'the menu survived the repaint').toContain('q› Quote');

      const button = where('q› Quote');
      pointer(0, 'M', button.x + 1, button.y);
      pointer(0, 'm', button.x + 1, button.y);
      await fixture.waitForCapture((current) => /hello there\s+×/.test(current), shell.pane);

      // Leave the draft, then the search filter, before quitting.
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture(
        (current) => current.includes('Esc clear filter') && /hello there\s+×/.test(current),
        shell.pane
      );
      expect(screen(), 'closing a draft must not deny the existing send').not.toContain(
        'Nothing sent.'
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture((current) => !current.includes('Esc clear filter'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture((current) => current.includes('QUOTE_BOARD_EXIT=0'), shell.pane);
    });
  });

  it('keeps a chat selection, its Copy and its Quote while the agent is busy (#4344)', async () => {
    await withE2EFixture(async (fixture) => {
      const member = await squadWithMember(fixture);
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('busy-board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'tmt ops ui --squad product; echo BUSY_BOARD_EXIT=$?',
        'Enter',
      ]);
      await fixture.waitForCapture((screen) => screen.includes('OPS / UI'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, '1']);
      await fixture.waitForCapture((screen) => /\/[ \t]+Search/.test(screen), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, '/', 'auth-fix', 'Enter', 'End']);
      await fixture.waitForCapture(
        (screen) => screen.includes('⏎ to message auth-fix'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      fixture.tmux(['send-keys', '-t', shell.pane, '-l', 'first question']);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
      await fixture.waitForCapture((screen) => screen.includes('auth-fix: '), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);

      const screen = () => fixture.tmux(['capture-pane', '-p', '-t', shell.pane]);
      const where = (needle: string) => {
        const rows = screen().split('\n');
        const y = rows.findIndex((row) => row.includes(needle));
        let evidence = '';
        if (y < 0) {
          const attempts = requestAttempts(fixture);
          evidence = `\nStored attempts: ${JSON.stringify(attempts)}\nAfter store read:\n${screen()}`;
        }
        expect(y, `${needle} on screen:\n${rows.join('\n')}${evidence}`).toBeGreaterThanOrEqual(0);
        return { x: [...rows[y]!.slice(0, rows[y]!.indexOf(needle))].length, y };
      };
      const pointer = (code: number, final: 'M' | 'm', x: number, y: number) =>
        fixture.tmux([
          'send-keys',
          '-t',
          shell.pane,
          '-l',
          `\u001b[<${code};${x + 1};${y + 1}${final}`,
        ]);
      const buffer = () => {
        try {
          return fixture.tmux(['show-buffer']).trim();
        } catch {
          return '';
        }
      };
      // Select the sent question, as the user does, then let the agent work around it.
      const text = where('first question');
      pointer(0, 'M', text.x, text.y);
      pointer(32, 'M', text.x + 'first question'.length, text.y);
      pointer(0, 'm', text.x + 'first question'.length, text.y);
      await fixture.waitForCapture((current) => current.includes('q› Quote'), shell.pane);
      await fixture.waitFor(
        () => buffer() === 'first question',
        5_000,
        'the release copied the selection'
      );

      // A busy turn: the agent's finals land in the product's own store, one after another,
      // and the last one grows in place. Each lands as a board event between pointer events.
      const store = new Database(path.join(fixture.globalDir, 'tmt.db'));
      try {
        const agent = store.prepare('SELECT id FROM identities WHERE name = ?').get('auth-fix') as {
          id: string;
        };
        const land = store.prepare(
          "INSERT INTO recent_responses (identity_id, text, ended_ms, provider, truncated, session_id, expires_ms) VALUES (?, ?, ?, 'claude', 0, NULL, ?)"
        );
        const grow = store.prepare(
          'UPDATE recent_responses SET text = ? WHERE sequence = (SELECT max(sequence) FROM recent_responses WHERE identity_id = ?)'
        );
        const now = Date.now();
        for (let round = 1; round <= 4; round += 1) {
          land.run(agent.id, `step ${round} is done`, now + round, now + 3_600_000);
          await fixture.waitForCapture(
            (current) => current.includes(`step ${round} is done`),
            shell.pane
          );
          grow.run(`step ${round} is done and the agent keeps writing`, agent.id);
          // The Selection box floats over the middle of the grown row, so wait for its tail.
          await fixture.waitForCapture(
            (current) =>
              current.includes(`step ${round} is done`) && current.includes('keeps writing'),
            shell.pane
          );
          expect(screen(), `Quote stays after round ${round}:\n${screen()}`).toContain('q› Quote');
        }
      } finally {
        store.close();
      }
      expect(screen(), 'the selection is still on its text').toContain('first question');

      // Copy again through the reader's key, then Quote: both work mid-turn.
      fixture.tmux(['set-buffer', 'stale']);
      fixture.tmux(['send-keys', '-t', shell.pane, '-l', 'y']);
      await fixture.waitFor(
        () => buffer() === 'first question',
        5_000,
        `y copied during the busy turn:\n${screen()}`
      );
      const button = where('q› Quote');
      pointer(0, 'M', button.x + 1, button.y);
      pointer(0, 'm', button.x + 1, button.y);
      await fixture.waitForCapture((current) => /first question\s+×/.test(current), shell.pane);

      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture(
        (current) => current.includes('Esc clear filter') && /first question\s+×/.test(current),
        shell.pane
      );
      expect(screen(), 'closing a draft must not deny the existing send').not.toContain(
        'Nothing sent.'
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture((current) => !current.includes('Esc clear filter'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture((current) => current.includes('BUSY_BOARD_EXIT=0'), shell.pane);
    });
  }, 60_000);

  it('opens a chat link in a popover that copies it, shows it whole and runs the opener once (#4377)', async () => {
    await withE2EFixture(async (fixture) => {
      await squadWithMember(fixture);
      // The opener records its argv instead of starting a browser: the link must arrive as
      // one argument, whatever characters it holds.
      const log = path.join(fixture.root, 'opener.log');
      const recorder = path.join(fixture.root, 'record-open');
      fs.writeFileSync(recorder, `#!/bin/sh\nprintf '%s\\n' "$#" "$@" >> '${log}'\n`, {
        mode: 0o755,
      });
      fs.appendFileSync(
        path.join(fixture.globalDir, 'ops.toml'),
        `\nopener = [${JSON.stringify(recorder)}, "{url}"]\n`
      );
      const link = "https://example.com/a%20b;c?x='1'&y=2#frag";
      const store = new Database(path.join(fixture.globalDir, 'tmt.db'));
      try {
        const agent = store.prepare('SELECT id FROM identities WHERE name = ?').get('auth-fix') as {
          id: string;
        };
        const now = Date.now();
        store
          .prepare(
            "INSERT INTO recent_responses (identity_id, text, ended_ms, provider, truncated, session_id, expires_ms) VALUES (?, ?, ?, 'claude', 0, NULL, ?)"
          )
          .run(agent.id, `Read [the docs](${link}) first.`, now, now + 3_600_000);
      } finally {
        store.close();
      }
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('link-board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'tmt ops ui --squad product; echo LINK_BOARD_EXIT=$?',
        'Enter',
      ]);
      await fixture.waitForCapture((screen) => screen.includes('OPS / UI'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, '1']);
      await fixture.waitForCapture((screen) => /\/[ \t]+Search/.test(screen), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, '/', 'auth-fix', 'Enter', 'End']);
      await fixture.waitForCapture(
        (screen) => screen.includes('⏎ to message auth-fix'),
        shell.pane
      );
      await fixture.waitForCapture((screen) => screen.includes('Read the docs first.'), shell.pane);

      const screen = () => fixture.tmux(['capture-pane', '-p', '-t', shell.pane]);
      const where = (needle: string) => {
        const rows = screen().split('\n');
        const y = rows.findIndex((row) => row.includes(needle));
        expect(y, `${needle} on screen:\n${screen()}`).toBeGreaterThanOrEqual(0);
        return { x: [...rows[y]!.slice(0, rows[y]!.indexOf(needle))].length, y };
      };
      const click = (x: number, y: number) => {
        for (const final of ['M', 'm'])
          fixture.tmux([
            'send-keys',
            '-t',
            shell.pane,
            '-l',
            `\u001b[<0;${x + 1};${y + 1}${final}`,
          ]);
      };
      const buffer = () => {
        try {
          return fixture.tmux(['show-buffer']).trim();
        } catch {
          return '';
        }
      };
      const open = async () => {
        const label = where('the docs');
        click(label.x + 1, label.y);
        await fixture.waitForCapture((current) => current.includes('┌ Link'), shell.pane);
      };

      // The popover names where the link goes before it offers anything.
      await open();
      const popover = screen();
      expect(popover).toContain('https://example.com');
      for (const action of ['Open', 'Copy', 'Show full'])
        expect(popover, popover).toContain(action);

      // Copy puts the whole address, not the label, on the clipboard.
      fixture.tmux(['send-keys', '-t', shell.pane, '-l', 'y']);
      await fixture.waitFor(() => buffer() === link, 5_000, `Copy:\n${screen()}`);

      // Show full lists every character of the address.
      await open();
      fixture.tmux(['send-keys', '-t', shell.pane, '-l', 'f']);
      await fixture.waitForCapture((current) => current.includes('Full link'), shell.pane);
      const lines = screen()
        .split('\n')
        .map((row) => /│ [› ]\s?(\S.*?)\s*│\s*$/.exec(row)?.[1])
        .filter((row): row is string => Boolean(row));
      expect(lines.join('').replace(/\s/g, ''), screen()).toContain(link);
      expect(screen()).toContain('Enter copies · Esc back');
      // Esc goes back to the popover; a second Esc closes it.
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture(
        (current) => current.includes('┌ Link') && !current.includes('Full link'),
        shell.pane
      );
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      await fixture.waitForCapture((current) => !current.includes('┌ Link'), shell.pane);

      // Open runs the configured command with the address as one argument, once.
      await open();
      fixture.tmux(['send-keys', '-t', shell.pane, '-l', 'o']);
      await fixture.waitFor(() => fs.existsSync(log), 10_000, 'the opener ran');
      expect(fs.readFileSync(log, 'utf8')).toBe(`1\n${link}\n`);

      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture((current) => current.includes('LINK_BOARD_EXIT=0'), shell.pane);
    });
  }, 90_000);

  it('shows the selected agent live screen, with its colours, and stops reading when left (#3536)', async () => {
    await withE2EFixture(async (fixture) => {
      // The agent pane prints a coloured line, then a counter that changes every second.
      const member = await squadWithMember(
        fixture,
        `sh -c 'printf "\\033[31mred words\\033[0m plain\\n"; i=0; while true; do i=$((i+1)); echo "tick $i"; sleep 1; done'`
      );
      await fixture.attachSessionClient('e2e');
      const shell = fixture.createShellPane('screen-board');
      fixture.tmux(['select-window', '-t', shell.pane]);
      fixture.tmux([
        'send-keys',
        '-t',
        shell.pane,
        'tmt ops ui --squad product; echo SCREEN_BOARD_EXIT=$?',
        'Enter',
      ]);
      await fixture.waitForCapture((screen) => screen.includes('OPS / UI'), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, '1']);
      await fixture.waitForCapture((screen) => /\/[ \t]+Search/.test(screen), shell.pane);
      fixture.tmux(['send-keys', '-t', shell.pane, '/', 'auth-fix', 'Enter', 'End']);
      await fixture.waitForCapture(
        (screen) => screen.includes('⏎ to message auth-fix'),
        shell.pane
      );
      const screen = () => fixture.tmux(['capture-pane', '-p', '-t', shell.pane]);
      const colored = () => fixture.tmux(['capture-pane', '-p', '-e', '-t', shell.pane]);

      // Off by default: the chat, not the pane.
      expect(screen()).not.toContain('red words');

      // l on the Talk pane shows the live screen.
      fixture.tmux(['send-keys', '-t', shell.pane, 'Tab']);
      fixture.tmux(['send-keys', '-t', shell.pane, '-l', 'l']);
      await fixture.waitForCapture((current) => current.includes('Live screen'), shell.pane);
      await fixture.waitFor(
        () => screen().includes('red words plain'),
        10_000,
        `the pane screen:\n${screen()}`
      );
      // The colour survived: the word sits after a colour escape that its neighbours lack.
      const row = colored()
        .split('\n')
        .find((line) => line.includes('red words'))!;
      expect(row, row).toMatch(/\u001b\[(?:[0-9;]*;)?(?:31|38;5;1)m(?:\u001b\[[0-9;]*m)*red words/);
      expect(row).not.toMatch(/(?:31|38;5;1)m(?:\u001b\[[0-9;]*m)*[^\u001b]*plain/);

      // It follows the pane: the counter moves within a few polls.
      const tick = () => [...screen().matchAll(/tick (\d+)/g)].at(-1)?.[1];
      const first = await (async () => {
        await fixture.waitFor(() => tick() !== undefined, 10_000, `a tick:\n${screen()}`);
        return Number(tick());
      })();
      await fixture.waitFor(() => Number(tick()) > first, 10_000, `the screen moves:\n${screen()}`);

      // l again returns to the chat and the reads stop.
      fixture.tmux(['send-keys', '-t', shell.pane, '-l', 'l']);
      await fixture.waitForCapture((current) => !current.includes('Live screen'), shell.pane);
      expect(screen()).not.toContain('red words');
      void member;

      fixture.tmux(['send-keys', '-t', shell.pane, 'Escape']);
      fixture.tmux(['send-keys', '-t', shell.pane, 'q']);
      await fixture.waitForCapture(
        (current) => current.includes('SCREEN_BOARD_EXIT=0'),
        shell.pane
      );
    });
  }, 90_000);

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
