import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import type { E2EFixture } from './harness.js';

/** Owns the public-template child's lifetime before the fixture removes its stores. */
export function savedMemberLifetime(fixture: E2EFixture, shim: string) {
  const word = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
  const cli = resolveCliExecutables().cli;
  const squad = path.join(path.dirname(cli.executable), 'tmt-ops');
  const boardFile = path.join(shim, 'board.json');
  const opsFile = path.join(shim, 'ops.json');
  const launcherFile = path.join(shim, 'launcher.json');
  const providerFile = path.join(shim, 'provider.json');
  const entered = path.join(shim, 'exit-entered');
  const gate = path.join(shim, 'exit-release');
  const observer = path.join(shim, 'tmt');
  writeExecutable(
    observer,
    `#!/bin/sh
case "$1 $2 $3" in
  'ops ui ') record=${word(boardFile)} ;;
  'run --save new-worker') record=${word(launcherFile)} ;;
  *) record= ;;
esac
if [ -n "$record" ]; then
  printf '{"pid":%s,"pane":"%s"}\\n' "$$" "$TMUX_PANE" > "$record.tmp"
  mv "$record.tmp" "$record"
fi
exec ${[cli.executable, ...cli.args].map(word).join(' ')} "$@"
`
  );
  // Keep the public extension dispatch and real Core reads. The observer execs
  // Core, so its recorded PID is the launcher, rather than a waiting wrapper.
  const extension = path.join(fixture.wrapperDir, 'tmt-ops');
  fs.unlinkSync(extension);
  writeExecutable(
    extension,
    `#!/bin/sh
printf '{"pid":%s,"pane":"%s"}\\n' "$$" "$TMUX_PANE" > ${word(opsFile + '.tmp')}
mv ${word(opsFile + '.tmp')} ${word(opsFile)}
export TMT_EXECUTABLE=${word(observer)}
exec ${word(squad)} "$@"
`
  );
  const input = path.join(fixture.root, 'new-member-input');
  writeExecutable(
    path.join(shim, 'ops-new-peer'),
    `#!/bin/sh
printf '{"pid":%s,"launcher":%s}\\n' "$$" "$PPID" > ${word(providerFile + '.tmp')}
mv ${word(providerFile + '.tmp')} ${word(providerFile)}
printf 'NEW_MEMBER_READY\\n'
while IFS= read -r line; do
  printf '%s\\n' "$line" >> ${word(input)}
  printf 'NEW_MEMBER_REPLY:%s\\n' "$line"
done
trap '' HUP
printf 'entered\\n' > ${word(entered)}
IFS= read -r release < ${word(gate)}
`
  );
  execFileSync('mkfifo', [gate]);
  const gateFd = fs.openSync(gate, fs.constants.O_RDWR | fs.constants.O_NONBLOCK);
  const record = (file: string): { pid: number; pane?: string; launcher?: number } | undefined => {
    if (!fs.existsSync(file)) return undefined;
    const value = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (!Number.isSafeInteger(value.pid) || value.pid <= 1) {
      throw new Error(`Invalid owned process record: ${file}`);
    }
    if (value.pane !== undefined && !/^%\d+$/.test(value.pane)) {
      throw new Error(`Invalid owned pane record: ${file}`);
    }
    return value;
  };
  const gone = (pids: number[]) => pids.every((pid) => !fixture.mockProcessIsRunning(pid));
  let released = false;
  let boardPane: string | undefined;
  let cleanup: Promise<number[]> | undefined;
  return {
    entered,
    launcher: () => record(launcherFile),
    provider: () => record(providerFile),
    board(pane: string) {
      boardPane = pane;
    },
    release() {
      if (!released) fs.writeSync(gateFd, 'release\n');
      released = true;
    },
    close() {
      fs.closeSync(gateFd);
    },
    finish() {
      return (cleanup ??= (async () => {
        const owned = new Set<number>();
        const collect = () => {
          for (const file of [boardFile, opsFile, launcherFile, providerFile]) {
            const value = record(file);
            if (value) owned.add(value.pid);
          }
          return [...owned];
        };
        try {
          // Stop and join the board first: an interrupted form cannot start a
          // member after this scenario has taken its final ownership inventory.
          const board = [record(boardFile), record(opsFile)].flatMap((value) =>
            value ? [value.pid] : []
          );
          collect();
          if (boardPane && (board.length === 0 || !gone(board))) {
            fixture.tmux(['send-keys', '-t', boardPane, 'C-c']);
            await fixture.waitFor(() => gone(board), 2_000, 'saved-member board exit');
          }
          const pane = fixture
            .tmux(['list-panes', '-a', '-F', '#{session_name}|#{pane_id}|#{pane_pid}'])
            .trim()
            .split('\n')
            .find((line) => line.startsWith('product|'))
            ?.split('|');
          const panePid = pane ? Number(pane[2]) : undefined;
          if (panePid) owned.add(panePid);
          if (panePid && !record(launcherFile)) {
            await fixture.waitFor(
              () => record(launcherFile) !== undefined || gone([panePid]),
              2_000,
              'partial member startup ownership'
            );
          }
          const launcher = record(launcherFile);
          if (launcher && !gone([launcher.pid])) {
            fixture.tmux(['send-keys', '-t', launcher.pane!, 'C-d']);
          }
          await fixture.waitFor(
            () => gone(collect()),
            2_000,
            'saved-member launcher and provider exit'
          );
        } catch (error) {
          // A failed scenario still owns these exact PIDs; never let fixture
          // deletion race a late launcher write, even when graceful exit fails.
          for (const pid of collect()) if (!gone([pid])) process.kill(pid, 'SIGKILL');
          await fixture.waitFor(
            () => gone(collect()),
            2_000,
            'failed saved-member process cleanup'
          );
          throw error;
        }
        return collect();
      })());
    },
  };
}
