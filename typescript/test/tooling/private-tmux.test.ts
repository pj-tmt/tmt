import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
} from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';
import {
  assertPrivateSocket,
  hostSocketDirectories,
  privateTmux,
  PrivateTmuxRefused,
} from '../support/private-tmux.js';

const uid = process.getuid!();

interface Fixture {
  root: string;
  stub: string;
  /** Each stub call: its argv and whether TMUX/TMUX_PANE reached it. */
  calls: () => { argv: string; tmux: string; pane: string }[];
}

/** A task-owned root and a stub `tmux`, so no case can start or reach a real server. */
async function withFixture(run: (fixture: Fixture) => void | Promise<void>) {
  // /tmp, not os.tmpdir(): macOS's per-user temp directory is too long for a Unix socket path.
  const root = realpathSync(mkdtempSync('/tmp/ptmux-'));
  try {
    const log = path.join(root, 'calls.log');
    const stub = path.join(root, 'stub-tmux');
    writeExecutable(
      stub,
      `#!/bin/sh\nprintf '%s\\t%s\\t%s\\n' "$*" "\${TMUX-unset}" "\${TMUX_PANE-unset}" >> "${log}"\n[ "\${STUB_FAIL-}" = 1 ] && { echo boom >&2; exit 1; }\nexit 0\n`,
      0o755
    );
    await run({
      root,
      stub,
      calls: () =>
        existsSync(log)
          ? readFileSync(log, 'utf8')
              .trim()
              .split('\n')
              .map((line) => {
                const [argv, tmux, pane] = line.split('\t');
                return { argv, tmux, pane };
              })
          : [],
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const options = (fixture: Fixture, extra: Record<string, unknown> = {}) => ({
  root: fixture.root,
  name: 'probe',
  env: { PATH: '/usr/bin:/bin' } as NodeJS.ProcessEnv,
  executable: fixture.stub,
  ambient: {} as NodeJS.ProcessEnv,
  ...extra,
});

describe('privateTmux', () => {
  it('selects its own socket with an explicit -S and strips TMUX and TMUX_PANE', async () => {
    await withFixture((fixture) => {
      const env = {
        PATH: '/usr/bin:/bin',
        TMUX: `/private/tmp/tmux-${uid}/default,1,0`,
        TMUX_PANE: '%9',
      };
      const server = privateTmux(options(fixture, { env }));
      expect(server.socket).toBe(path.join(fixture.root, 'probe.sock'));
      server.tmux(['new-session', '-d', '-s', 'x']);
      server.run(['-u', 'list-sessions']);
      expect(fixture.calls()).toEqual([
        {
          argv: `-f /dev/null -S ${server.socket} new-session -d -s x`,
          tmux: 'unset',
          pane: 'unset',
        },
        { argv: `-f /dev/null -S ${server.socket} -u list-sessions`, tmux: 'unset', pane: 'unset' },
      ]);
      // The caller's own environment object is untouched, and is read again on each call.
      expect(env.TMUX).toBeDefined();
    });
  });

  it('reports a failing command and stops without failing when the server is already gone', async () => {
    await withFixture((fixture) => {
      const env = { PATH: '/usr/bin:/bin', STUB_FAIL: '1' };
      const server = privateTmux(options(fixture, { env, config: '/some/tmux.conf' }));
      expect(() => server.tmux(['list-sessions'])).toThrow('Private tmux failed: boom');
      expect(() => server.stop()).not.toThrow();
      expect(fixture.calls().map((call) => call.argv)).toEqual([
        `-f /some/tmux.conf -S ${server.socket} list-sessions`,
        `-f /some/tmux.conf -S ${server.socket} kill-server`,
      ]);
    });
  });

  it('refuses a socket that is not private before any tmux command runs', async () => {
    await withFixture((fixture) => {
      const host = path.join(fixture.root, 'host');
      mkdirSync(path.join(host, `tmux-${uid}`), { recursive: true });
      const refusals: [string, () => unknown, string][] = [
        [
          'a name that leaves the root',
          () => privateTmux(options(fixture, { name: '../escape' })),
          'outside the test root',
        ],
        [
          'a root that is a host socket directory ($TMUX_TMPDIR)',
          () =>
            privateTmux(
              options(fixture, {
                root: path.join(host, `tmux-${uid}`),
                ambient: { TMUX_TMPDIR: host },
              })
            ),
          'host tmux socket directory',
        ],
        [
          'a root holding the socket $TMUX names',
          () =>
            privateTmux(
              options(fixture, {
                root: host,
                ambient: { TMUX: `${host}/default,1,0` },
              })
            ),
          'host tmux socket directory',
        ],
        [
          'a root that does not exist',
          () => privateTmux(options(fixture, { root: path.join(fixture.root, 'missing') })),
          'existing absolute directory',
        ],
        [
          'a path too long for a socket',
          () => privateTmux(options(fixture, { name: 'x'.repeat(120) })),
          'too long',
        ],
      ];
      for (const [label, attempt, needle] of refusals) {
        expect(attempt, label).toThrow(PrivateTmuxRefused);
        expect(attempt, label).toThrow(needle);
      }
      const linked = path.join(fixture.root, 'linked');
      mkdirSync(linked);
      symlinkSync(`/private/tmp/tmux-${uid}/default`, path.join(linked, 'probe.sock'));
      expect(() => privateTmux(options(fixture, { root: linked }))).toThrow('is a symlink');
      expect(fixture.calls(), 'a refusal never reaches tmux').toEqual([]);
    });
  });

  it('refuses a global option that would choose another server', async () => {
    await withFixture((fixture) => {
      const server = privateTmux(options(fixture));
      for (const args of [
        ['-S', '/x', 'kill-server'],
        ['-L', 'default', 'kill-server'],
        ['-f', '/x', 'kill-server'],
        ['-c', 'x', 'kill-server'],
        ['-uS', 'kill-server'],
      ])
        expect(() => server.run(args), args.join(' ')).toThrow('is not allowed before the command');
      // Options after the command belong to it: capture-pane -S is a line number.
      server.run(['capture-pane', '-p', '-S', '-5']);
      expect(fixture.calls().map((call) => call.argv)).toEqual([
        `-f /dev/null -S ${server.socket} capture-pane -p -S -5`,
      ]);
    });
  });

  it('notices a socket replaced by a symlink after construction', async () => {
    await withFixture((fixture) => {
      const server = privateTmux(options(fixture));
      symlinkSync(`/private/tmp/tmux-${uid}/default`, server.socket);
      expect(() => server.stop()).toThrow('is a symlink');
      expect(fixture.calls()).toEqual([]);
    });
  });
});

describe('hostSocketDirectories', () => {
  it('names the default directories and the ones the ambient environment points at', async () => {
    await withFixture((fixture) => {
      const directories = hostSocketDirectories(
        { TMUX_TMPDIR: fixture.root, TMUX: `${fixture.root}/elsewhere/default,5,1` },
        uid
      );
      expect(directories).toEqual(
        expect.arrayContaining([
          realpathSync('/tmp') + `/tmux-${uid}`,
          path.join(fixture.root, `tmux-${uid}`),
          path.join(fixture.root, 'elsewhere'),
        ])
      );
      expect(() => assertPrivateSocket(`/tmp/tmux-${uid}/probe.sock`, fixture.root, {})).toThrow(
        PrivateTmuxRefused
      );
    });
  });
});
