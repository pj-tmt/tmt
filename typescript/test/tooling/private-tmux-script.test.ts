import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
  symlinkSync,
} from 'node:fs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';

const script = fileURLToPath(new URL('../../../scripts/private-tmux.sh', import.meta.url));
const uid = process.getuid!();

/** The host tools the script runs; everything else, tmux included, is a fake. */
const hostTools = ['id', 'dirname', 'basename', 'mkdir', 'chmod', 'env'].map((name) => {
  const found = spawnSync('/bin/sh', ['-c', `command -v ${name}`], { encoding: 'utf8' });
  if (found.status !== 0) throw new Error(`The private-tmux fixture needs ${name}.`);
  return [name, found.stdout.trim()] as const;
});

interface Fixture {
  root: string;
  calls: () => string[];
}

/**
 * A task-owned root with a fake `tmux` as the only tmux on PATH. The fake records its argv
 * and whether TMUX/TMUX_PANE reached it, so no case can start or reach a real server.
 */
async function withFixture(run: (fixture: Fixture) => void | Promise<void>) {
  const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'private-tmux-')));
  try {
    const bin = path.join(root, 'fake-bin');
    mkdirSync(bin);
    const log = path.join(root, 'calls.log');
    writeExecutable(
      path.join(bin, 'tmux'),
      `#!/bin/sh\nprintf 'argv %s\\n' "$*" >> "${log}"\nprintf 'env TMUX=%s TMUX_PANE=%s\\n' "\${TMUX-unset}" "\${TMUX_PANE-unset}" >> "${log}"\n`,
      0o755
    );
    for (const [name, target] of hostTools) symlinkSync(target, path.join(bin, name));
    await run({
      root,
      calls: () => (existsSync(log) ? readFileSync(log, 'utf8').trim().split('\n') : []),
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

function privateTmux(fixture: Fixture, args: string[], env: Record<string, string> = {}) {
  const result = spawnSync('/bin/sh', [script, ...args], {
    env: { PATH: path.join(fixture.root, 'fake-bin'), ...env },
    encoding: 'utf8',
    timeout: 10_000,
  });
  return { status: result.status, stdout: result.stdout, stderr: result.stderr };
}

function refused(result: ReturnType<typeof privateTmux>, fixture: Fixture, needle: string) {
  expect(result.status, result.stderr).toBe(2);
  expect(result.stderr).toContain(`private-tmux: `);
  expect(result.stderr).toContain(needle);
  expect(fixture.calls(), 'a refusal never reaches tmux').toEqual([]);
}

describe('scripts/private-tmux.sh', () => {
  it('drives tmux on its own fixed socket with TMUX and TMUX_PANE removed', async () => {
    await withFixture(async (fixture) => {
      const scratch = path.join(fixture.root, 'scratch', 'one');
      // The pane the agent runs in names the host server; the command must not see it.
      const result = privateTmux(fixture, [scratch, 'new-session', '-d', '-s', 'probe'], {
        TMUX: `/private/tmp/tmux-${uid}/default,1,0`,
        TMUX_PANE: '%9',
      });
      expect(result.status, result.stderr).toBe(0);
      expect(fixture.calls()).toEqual([
        `argv -f /dev/null -S ${scratch}/tmux.sock new-session -d -s probe`,
        'env TMUX=unset TMUX_PANE=unset',
      ]);
      expect(statSync(scratch).mode & 0o777, 'a directory it creates is private').toBe(0o700);

      // The same directory is reused, and kill-server is reachable only on that socket.
      const stop = privateTmux(fixture, [scratch, '-u', 'kill-server']);
      expect(stop.status, stop.stderr).toBe(0);
      expect(fixture.calls().at(-2)).toBe(
        `argv -u -f /dev/null -S ${scratch}/tmux.sock kill-server`
      );
    });
  });

  it('leaves an existing directory as it is', async () => {
    await withFixture(async (fixture) => {
      const scratch = path.join(fixture.root, 'existing');
      mkdirSync(scratch, { mode: 0o755 });
      expect(privateTmux(fixture, [scratch, 'list-sessions']).status).toBe(0);
      expect(statSync(scratch).mode & 0o777).toBe(0o755);
    });
  });

  it('refuses a directory inside a host tmux socket directory before creating anything', async () => {
    await withFixture(async (fixture) => {
      const host = path.join(fixture.root, 'host');
      mkdirSync(host);
      // An ambient TMUX_TMPDIR and an ambient $TMUX each name a directory the host server uses.
      const viaTmpdir = path.join(host, `tmux-${uid}`, 'scratch');
      refused(
        privateTmux(fixture, [viaTmpdir, 'list-sessions'], { TMUX_TMPDIR: host }),
        fixture,
        'host tmux socket directory'
      );
      expect(existsSync(viaTmpdir)).toBe(false);

      const socketDir = path.join(fixture.root, 'elsewhere');
      const viaTmux = path.join(socketDir, 'inside');
      refused(
        privateTmux(fixture, [viaTmux, 'list-sessions'], { TMUX: `${socketDir}/default,1,0` }),
        fixture,
        'host tmux socket directory'
      );
      expect(existsSync(viaTmux)).toBe(false);

      // The directories tmux uses by default, however the path is spelled.
      for (const base of [`/tmp/tmux-${uid}`, `/private/tmp/tmux-${uid}`]) {
        const probe = path.join(base, `private-tmux-probe-${process.pid}`);
        refused(
          privateTmux(fixture, [probe, 'kill-server']),
          fixture,
          'host tmux socket directory'
        );
        expect(existsSync(probe), probe).toBe(false);
      }
      const dotted = `/tmp/./tmux-${uid}/../tmux-${uid}/private-tmux-probe-${process.pid}`;
      refused(privateTmux(fixture, [dotted, 'kill-server']), fixture, '.. component');
      expect(existsSync(dotted)).toBe(false);
    });
  });

  it('refuses any option that would choose another server', async () => {
    await withFixture(async (fixture) => {
      const scratch = path.join(fixture.root, 'scratch');
      for (const option of [['-S', '/x'], ['-L', 'default'], ['-f', '/x'], ['-c', 'x'], ['-uS']]) {
        refused(
          privateTmux(fixture, [scratch, ...option, 'kill-server']),
          fixture,
          'is not allowed before the command'
        );
      }
      expect(existsSync(scratch)).toBe(false);
    });
  });

  it('refuses a relative directory, a missing command, a symlinked socket and a long path', async () => {
    await withFixture(async (fixture) => {
      refused(privateTmux(fixture, ['relative', 'list-sessions']), fixture, 'absolute path');
      // mkdir -p would create `gone` and then climb out of it into the host's directory.
      const climbing = `/tmp/private-tmux-gone-${process.pid}/../tmux-${uid}/probe`;
      refused(privateTmux(fixture, [climbing, 'list-sessions']), fixture, '.. component');
      expect(existsSync(`/tmp/private-tmux-gone-${process.pid}`)).toBe(false);
      refused(
        privateTmux(fixture, [path.join(fixture.root, 's'), '-u']),
        fixture,
        'no tmux command'
      );
      refused(privateTmux(fixture, [path.join(fixture.root, 's')]), fixture, 'usage');

      const linked = path.join(fixture.root, 'linked');
      mkdirSync(linked);
      symlinkSync(`/private/tmp/tmux-${uid}/default`, path.join(linked, 'tmux.sock'));
      refused(privateTmux(fixture, [linked, 'kill-server']), fixture, 'is a symlink');

      const long = path.join(fixture.root, 'x'.repeat(120));
      refused(privateTmux(fixture, [long, 'list-sessions']), fixture, 'too long');
      expect(existsSync(long)).toBe(false);
    });
  });
});
