import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import {
  decide,
  hostTmuxViolations,
  resolveTmuxCommand,
  simpleCommands,
} from '../../scripts/host-tmux-guard.mjs';

// The project hook that keeps an agent's Bash tool from ending or reconfiguring the host tmux
// server (#4189). The environment and uid are injected, so no test touches tmux.

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const script = fileURLToPath(new URL('../../scripts/host-tmux-guard.mjs', import.meta.url));
const uid = 501;
const refused = (command: string, env: Record<string, string> = {}) =>
  hostTmuxViolations(command, env, uid).map((violation) => violation.command);

describe('commands that reach the host server', () => {
  it.each([
    'tmux kill-server',
    'tmux kill-session -t work',
    'tmux kill-ser',
    'tmux killw -t work',
    'tmux kill-window -t work',
    'tmux kill-window -a -t =work:=lead',
    'tmux kill-pane -t %3',
    'tmux killp -t %3',
    'tmux set -g mouse on',
    'tmux set-option -s escape-time 0',
    'tmux set-option -gu status',
    'tmux setw -g aggressive-resize on',
    'tmux set-hook -g after-new-window "run-shell true"',
    'tmux set-hook -t work pane-died "run-shell true"',
  ])('are refused: %s', (command) => {
    expect(refused(command)).toHaveLength(1);
  });

  it.each([
    '/opt/homebrew/bin/tmux kill-server',
    'tmux -2 kill-server',
    'tmux -L default kill-server',
    'tmux -L work kill-session -t a',
    'tmux -Lwork kill-server',
    'tmux -f /dev/null kill-server',
    'env TERM=xterm tmux kill-server',
    'env -u TMUX tmux kill-server',
    'FOO=1 tmux kill-server',
    'sudo -u builder tmux kill-server',
    'timeout 5 tmux kill-server',
    'command tmux kill-server',
    'cd /tmp && tmux kill-server',
    'ls; tmux kill-server',
    'echo y | tmux kill-server',
    'true || tmux kill-server',
    'if tmux has-session; then tmux kill-server; fi',
    'for s in a b; do tmux kill-session -t $s; done',
    'ls $(tmux kill-server)',
    'ls `tmux kill-server`',
    'echo "$(tmux kill-server)"',
    "bash -c 'tmux kill-server'",
    'sh -lc "tmux kill-server"',
    'env bash -c "tmux kill-server"',
    'bash -c "bash -c \'tmux kill-server\'"',
    'eval "tmux kill-server"',
    'tmux new-session -d \\; kill-server',
    "tmux new-session -d ';' kill-server",
    'printf x | xargs -I{} tmux kill-session -t {}',
  ])('are refused through any wrapper or shell: %s', (command) => {
    expect(refused(command), command).toHaveLength(1);
  });

  it('names each refused command with the reason and truncates a long one', () => {
    expect(hostTmuxViolations('tmux kill-server', {}, uid)).toEqual([
      { command: 'tmux kill-server', reason: "tmux kill-server stops the host server's agents" },
    ]);
    const long = `tmux kill-session -t ${'x'.repeat(200)}`;
    const [violation] = hostTmuxViolations(long, {}, uid);
    expect(violation.command).toHaveLength(100);
    expect(violation.command.endsWith('...')).toBe(true);
  });

  it('refuses every chained command separately', () => {
    expect(refused('tmux kill-server; tmux set -g mouse on')).toHaveLength(2);
    expect(refused('tmux kill-window -t a \\; kill-pane -t b')).toHaveLength(2);
  });

  it('refuses an explicit socket that is a host socket', () => {
    expect(refused('tmux -S /tmp/tmux-501/default kill-server')).toHaveLength(1);
    expect(refused('tmux -S /private/tmp/tmux-501/work kill-server')).toHaveLength(1);
    expect(refused('tmux -S/tmp/tmux-501/default kill-server')).toHaveLength(1);
    expect(refused('tmux -S /tmp/x/../tmux-501/default kill-server')).toHaveLength(1);
    expect(
      refused('tmux -S /var/run/user/501/tmux-501/default kill-server', {
        TMUX_TMPDIR: '/var/run/user/501',
      })
    ).toHaveLength(1);
    // The socket the agent's own pane runs on, by whatever directory it lives in.
    const env = { TMUX: '/srv/agents/team.sock,4242,0' };
    expect(refused('tmux -S /srv/agents/team.sock kill-server', env)).toHaveLength(1);
    expect(refused('tmux -S /srv/agents/other.sock kill-server', env)).toHaveLength(1);
    expect(refused('tmux -S /srv/scratch/own.sock kill-server', env)).toEqual([]);
  });
});

describe('commands that stay allowed', () => {
  it.each([
    // A private scratch server: an explicit -S socket that is not a host socket.
    'env -u TMUX -u TMUX_PANE tmux -S /tmp/ptmux-x/tmux.sock kill-server',
    'tmux -S /tmp/ptmux-x/tmux.sock kill-session -t scratch',
    'tmux -S /tmp/ptmux-x/tmux.sock set -g mouse on',
    'tmux -S /tmp/ptmux-x/tmux.sock set-hook -g after-new-window true',
    'tmux -S "$SCRATCH/tmux.sock" kill-server',
    'tmux -S "$(mktemp -d)/tmux.sock" kill-server',
    'scripts/private-tmux.sh /tmp/ptmux-x kill-server',
    'sh scripts/private-tmux.sh /tmp/ptmux-x -u new-session -d',
    // Reading, typing into an explicit pane, and a lead's own windows.
    'tmux list-sessions',
    'tmux ls -F "#S"',
    'tmux list-panes -a',
    'tmux capture-pane -p -t %3',
    "tmux send-keys -t %3 'echo hi' Enter",
    'tmux display-message -p "#S"',
    'tmux show-options -g',
    "tmux new-window -t '=squad:' -n worker",
    "tmux kill-window -t '=squad:=worker'",
    "tmux killw -t '=squad:=worker'",
    // Options and hooks that belong to one session or window.
    'tmux set-option -t work status off',
    'tmux setw -t work:1 synchronize-panes on',
    // Text that only mentions the verbs.
    'echo "tmux kill-server"',
    "echo 'tmux kill-session -t work'",
    'git commit -m "fix: never run tmux kill-server on the host"',
    'grep -rn "kill-server" scripts',
    'gh pr create --body-file body.md --title "tmux kill-server guard"',
    'cat <<EOF\ntmux kill-server\nEOF',
    'git commit -m "$(cat <<\'EOF\'\nrefuse tmux kill-server\n\nand tmux set -g mouse on\nEOF\n)"',
    'cat <<-EOF\n\ttmux kill-server\n\tEOF\nls',
    '# tmux kill-server',
    'ls # tmux kill-server',
    'tmux kill-s',
    'tmux send-keys -t %3 "tmux kill-server" Enter',
    'tmux display-message "set -g mouse on"',
    "tmux send-keys -t %3 'echo done\\;' Enter",
    // An escaped semicolon is typed text, not the end of a tmux command.
    "tmux send-keys -t %3 'echo\\;' kill-server",
  ])('%s', (command) => {
    expect(refused(command), command).toEqual([]);
  });

  it('judges a heredoc body by its delimiter, not by its first line', () => {
    expect(refused('cat <<EOF\nEOF\ntmux kill-server')).toHaveLength(1);
    expect(refused("cat <<'END'\nEOF\ntmux kill-server\nEND\nls")).toEqual([]);
  });
});

describe('the shell reading', () => {
  it('splits a command line into simple commands with their quotes removed', () => {
    expect(simpleCommands('a b; c "d e" \'f g\' h\\ i && j | k\nl')).toEqual([
      ['a', 'b'],
      ['c', 'd e', 'f g', 'h i'],
      ['j'],
      ['k'],
      ['l'],
    ]);
    expect(simpleCommands('x "$(a b) c" `d e`')).toContainEqual(['a', 'b']);
    expect(simpleCommands('x "$(a b) c" `d e`')).toContainEqual(['d', 'e']);
    expect(simpleCommands('x # y z\nw')).toEqual([['x'], ['w']]);
    expect(simpleCommands("echo ''")).toEqual([['echo', '']]);
  });

  it('resolves a tmux command like tmux does', () => {
    expect(resolveTmuxCommand('killw')).toBe('kill-window');
    expect(resolveTmuxCommand('set')).toBe('set-option');
    expect(resolveTmuxCommand('kill-ser')).toBe('kill-server');
    expect(resolveTmuxCommand('kill-s')).toBeUndefined();
    expect(resolveTmuxCommand('set-b')).toBe('set-buffer');
    expect(resolveTmuxCommand('nonsense')).toBeUndefined();
    expect(resolveTmuxCommand('')).toBeUndefined();
  });
});

describe('the hook', () => {
  const payload = (command: unknown, tool = 'Bash') =>
    JSON.stringify({ tool_name: tool, tool_input: { command } });
  const run = (stdin: string) =>
    spawnSync(process.execPath, [script], {
      input: stdin,
      encoding: 'utf8',
      env: { PATH: process.env.PATH ?? '' },
    });

  it('blocks with a message that points to the rule and the private route', () => {
    const result = run(payload('tmux kill-server'));
    expect(result.status).toBe(2);
    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('Host tmux safety');
    expect(result.stderr).toContain('tmux kill-server');
    expect(result.stderr).toContain('scripts/private-tmux.sh');
    expect(result.stderr).toContain("kill-window -t '=<session>:=<name>'");
  });

  it('still blocks when started through a symlink to the script or to its directory', () => {
    // Node resolves import.meta.url through symlinks but leaves argv[1] as typed; a worktree opened
    // through a symlinked path (/tmp on macOS) must not turn the guard into a silent no-op.
    const directory = mkdtempSync(join(tmpdir(), 'host-tmux-guard-'));
    try {
      const toFile = join(directory, 'guard-link.mjs');
      symlinkSync(script, toFile);
      const toDirectory = join(directory, 'scripts-link');
      symlinkSync(dirname(script), toDirectory);
      for (const entry of [toFile, join(toDirectory, 'host-tmux-guard.mjs')]) {
        const result = spawnSync(process.execPath, [entry], {
          input: payload('tmux kill-server'),
          encoding: 'utf8',
          env: { PATH: process.env.PATH ?? '' },
        });
        expect(result.status, entry).toBe(2);
        expect(result.stderr, entry).toContain('Host tmux safety');
      }
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  it('lets an allowed command, another tool and a payload without a command through silently', () => {
    for (const stdin of [
      payload('tmux list-sessions'),
      payload('tmux -S /tmp/ptmux-x/tmux.sock kill-server'),
      payload('tmux kill-server', 'Read'),
      payload(undefined),
      payload(42),
      JSON.stringify({ tool_name: 'Bash' }),
    ]) {
      const result = run(stdin);
      expect(result.status, stdin).toBe(0);
      expect(result.stdout + result.stderr, stdin).toBe('');
    }
  });

  it('reports a payload it cannot read without blocking the command', () => {
    const result = run('not json');
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('could not check this command');
  });

  it('decides in process the same way', () => {
    expect(decide(JSON.parse(payload('tmux kill-server')), {}, uid)).toContain('Host tmux safety');
    expect(decide(JSON.parse(payload('ls')), {}, uid)).toBeUndefined();
  });

  it('is installed as the project PreToolUse hook for Bash and nothing else', () => {
    const settings = JSON.parse(readFileSync(`${repository}.claude/settings.json`, 'utf8'));
    expect(Object.keys(settings)).toEqual(['hooks']);
    expect(Object.keys(settings.hooks)).toEqual(['PreToolUse']);
    expect(settings.hooks.PreToolUse).toEqual([
      {
        matcher: 'Bash',
        hooks: [
          {
            type: 'command',
            command: 'node "$CLAUDE_PROJECT_DIR/typescript/scripts/host-tmux-guard.mjs"',
            timeout: 10,
          },
        ],
      },
    ]);
    expect(readFileSync(script, 'utf8')).toContain('PreToolUse');
  });
});
