import { spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { assertPrivateSocket, locateTmux, privateTmux } from '../support/private-tmux.js';

// #4580: every host publishes one launch template, the argv that opens a new pane in a session
// and runs one command there. The caller fills `{session}` and `{command}` and runs it. The tmux template
// is a constant of the CLI, so what it does to a real server is checked here by expanding the
// words `tmt host templates` printed and running them.
//
// The server is private to the sandbox: a PATH `tmux` forwards to it on an explicit socket that is
// proven private first, and TMUX and TMUX_PANE are removed from everything that runs. No host tmux
// is addressed, and only the server this test started is stopped.

interface Template {
  host: string;
  state: string;
  launch: { argv: string[]; placeholders: string[] } | null;
}

async function tmuxTemplate(sandbox: Sandbox): Promise<string[]> {
  const result = await runCli(sandbox, ['host', 'templates', '--json']);
  expect(result.status, result.stdout + result.stderr).toBe(0);
  const document = JSON.parse(result.stdout) as { version: number; hosts: Template[] };
  expect(document.version).toBe(1);
  const tmux = document.hosts.find((host) => host.host === 'tmux');
  expect(tmux).toMatchObject({ state: 'builtin' });
  expect(tmux!.launch!.placeholders).toEqual(['session', 'command']);
  return tmux!.launch!.argv;
}

/** Each placeholder fills exactly one word, as a caller does. */
function expand(argv: string[], session: string, command: string): string[] {
  return argv.map((word) =>
    word === '{session}' ? session : word === '{command}' ? command : word
  );
}

async function withPrivateServer(
  run: (world: {
    sandbox: Sandbox;
    tmux: (args: string[]) => string;
    launch: (session: string, command: string) => ReturnType<typeof spawnSync>;
  }) => Promise<void>
) {
  await withSandbox(async (sandbox) => {
    const real = locateTmux();
    const scratch = privateTmux({
      root: sandbox.root,
      name: 'launch',
      env: sandbox.env,
      executable: real,
    });
    const wrap = path.join(sandbox.root, 'wrap');
    mkdirSync(wrap);
    writeFileSync(
      path.join(wrap, 'tmux'),
      [
        '#!/bin/sh',
        'while [ "$#" -gt 0 ]; do case "$1" in -u|-2) shift ;; -S) shift 2 ;; *) break ;; esac; done',
        `exec '${real}' -f /dev/null -S '${scratch.socket}' -u "$@"`,
        '',
      ].join('\n')
    );
    chmodSync(path.join(wrap, 'tmux'), 0o700);
    const argv = await tmuxTemplate(sandbox);
    const env: NodeJS.ProcessEnv = {
      ...sandbox.env,
      PATH: [wrap, sandbox.env.PATH].join(path.delimiter),
    };
    delete env.TMUX;
    delete env.TMUX_PANE;
    try {
      await run({
        sandbox,
        tmux: (args) => scratch.tmux(args),
        launch: (session, command) => {
          // The wrapper selects the socket; prove it is private before the template reaches it.
          assertPrivateSocket(scratch.socket, sandbox.root);
          const words = expand(argv, session, command);
          return spawnSync(words[0]!, words.slice(1), { env, encoding: 'utf8', timeout: 10_000 });
        },
      });
    } finally {
      scratch.stop();
    }
  });
}

const OPENED = '#{@tmt.opened}';

describe('the tmux launch template', () => {
  it('opens a detached window in an existing session and marks only that pane', async () => {
    await withPrivateServer(async ({ tmux, launch }) => {
      tmux(['new-session', '-d', '-s', 'squad', '-x', '80', '-y', '24', 'sleep 600']);
      const first = tmux(['display-message', '-p', '-t', '=squad:', '#{pane_id}']);
      const active = tmux(['display-message', '-p', '-t', '=squad:', '#{window_id}']);

      const opened = launch('squad', 'sleep 301');
      expect(opened.status, String(opened.stderr)).toBe(0);
      const pane = String(opened.stdout).trim();
      expect(pane).toMatch(/^%\d+$/);

      const windows = tmux(['list-windows', '-t', '=squad:', '-F', '#{window_id}']).split('\n');
      expect(windows).toHaveLength(2);
      // Detached: the window the user is in stays current.
      expect(tmux(['display-message', '-p', '-t', '=squad:', '#{window_id}'])).toBe(active);
      expect(tmux(['display-message', '-p', '-t', pane, '#{pane_start_command}'])).toContain(
        'sleep 301'
      );
      expect(tmux(['display-message', '-p', '-t', pane, OPENED])).toBe('1');
      // A pane TMT did not open carries no marker.
      expect(tmux(['display-message', '-p', '-t', first, OPENED])).toBe('');
    });
  }, 60_000);

  it('creates a missing session detached with the command as its first window', async () => {
    await withPrivateServer(async ({ tmux, launch }) => {
      // The server needs one session to be reachable; the template must not touch it.
      tmux(['new-session', '-d', '-s', 'other', 'sleep 600']);
      const opened = launch('fresh', 'sleep 302');
      expect(opened.status, String(opened.stderr)).toBe(0);
      const pane = String(opened.stdout).trim();

      expect(tmux(['list-sessions', '-F', '#{session_name}']).split('\n').sort()).toEqual([
        'fresh',
        'other',
      ]);
      // No stray empty window beside the command's.
      expect(
        tmux(['list-windows', '-t', '=fresh:', '-F', '#{window_id}']).split('\n')
      ).toHaveLength(1);
      expect(tmux(['display-message', '-p', '-t', pane, '#{pane_start_command}'])).toContain(
        'sleep 302'
      );
      expect(tmux(['display-message', '-p', '-t', pane, OPENED])).toBe('1');
    });
  }, 60_000);

  it('hands the session and the command over as data, never as script text', async () => {
    await withPrivateServer(async ({ sandbox, tmux, launch }) => {
      tmux(['new-session', '-d', '-s', 'other', 'sleep 600']);
      const injected = path.join(sandbox.root, 'injected');
      const session = `a b$(touch ${injected})`;
      const opened = launch(session, `printf '%s' "$HOME" >/dev/null; sleep 303`);
      expect(opened.status, String(opened.stderr)).toBe(0);
      expect(existsSync(injected)).toBe(false);
      expect(tmux(['list-sessions', '-F', '#{session_name}'])).toContain('a b$(touch');
      const pane = String(opened.stdout).trim();
      // The command is a shell string the pane runs whole.
      expect(tmux(['display-message', '-p', '-t', pane, '#{pane_start_command}'])).toContain(
        'sleep 303'
      );
    });
  }, 60_000);

  it('keeps its marker when the pane is respawned', async () => {
    await withPrivateServer(async ({ tmux, launch }) => {
      tmux(['new-session', '-d', '-s', 'squad', 'sleep 600']);
      const opened = launch('squad', 'sleep 304');
      expect(opened.status, String(opened.stderr)).toBe(0);
      const pane = String(opened.stdout).trim();
      tmux(['respawn-pane', '-k', '-t', pane, 'sleep 305']);
      expect(tmux(['display-message', '-p', '-t', pane, '#{pane_start_command}'])).toContain(
        'sleep 305'
      );
      expect(tmux(['display-message', '-p', '-t', pane, OPENED])).toBe('1');
    });
  }, 60_000);

  it('refuses an empty session or command before it touches the server', async () => {
    await withPrivateServer(async ({ tmux, launch }) => {
      tmux(['new-session', '-d', '-s', 'other', 'sleep 600']);
      // tmux reads an empty session target as "any session", so the template must not pass it on.
      for (const [session, command] of [
        ['', 'sleep 306'],
        ['squad', ''],
      ] as const) {
        const refused = launch(session, command);
        expect(refused.status).toBe(2);
        expect(String(refused.stdout)).toBe('');
        expect(String(refused.stderr)).toContain('a session and a command are required');
      }
      expect(tmux(['list-sessions', '-F', '#{session_name}'])).toBe('other');
      expect(
        tmux(['list-windows', '-t', '=other:', '-F', '#{window_id}']).split('\n')
      ).toHaveLength(1);
    });
  }, 60_000);
});
