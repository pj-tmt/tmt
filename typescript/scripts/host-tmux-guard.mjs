// Claude Code PreToolUse hook for Bash: refuses a command that would run a destructive tmux
// command on the host server (#4189). It reads the hook payload on stdin; exit 2 blocks the
// command and sends stderr to the agent. A seatbelt for direct commands, not a sandbox: it
// cannot see inside a script the command runs or a shell variable that names the tmux binary.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// [name, alias]: tmux resolves a command by alias, by exact name, or by unique name prefix.
const TMUX_COMMANDS = [
  ['attach-session', 'attach'],
  ['bind-key', 'bind'],
  ['break-pane', 'breakp'],
  ['capture-pane', 'capturep'],
  ['choose-buffer'],
  ['choose-client'],
  ['choose-tree'],
  ['clear-history', 'clearhist'],
  ['clear-prompt-history', 'clearphist'],
  ['clock-mode'],
  ['command-prompt'],
  ['confirm-before', 'confirm'],
  ['copy-mode'],
  ['customize-mode'],
  ['delete-buffer', 'deleteb'],
  ['detach-client', 'detach'],
  ['display-menu', 'menu'],
  ['display-message', 'display'],
  ['display-panes', 'displayp'],
  ['display-popup', 'popup'],
  ['find-window', 'findw'],
  ['has-session', 'has'],
  ['if-shell', 'if'],
  ['join-pane', 'joinp'],
  ['kill-pane', 'killp'],
  ['kill-server'],
  ['kill-session'],
  ['kill-window', 'killw'],
  ['last-pane', 'lastp'],
  ['last-window', 'last'],
  ['link-window', 'linkw'],
  ['list-buffers', 'lsb'],
  ['list-clients', 'lsc'],
  ['list-commands', 'lscm'],
  ['list-keys', 'lsk'],
  ['list-panes', 'lsp'],
  ['list-sessions', 'ls'],
  ['list-windows', 'lsw'],
  ['load-buffer', 'loadb'],
  ['lock-client', 'lockc'],
  ['lock-server', 'lock'],
  ['lock-session', 'locks'],
  ['move-pane', 'movep'],
  ['move-window', 'movew'],
  ['new-session', 'new'],
  ['new-window', 'neww'],
  ['next-layout', 'nextl'],
  ['next-window', 'next'],
  ['paste-buffer', 'pasteb'],
  ['pipe-pane', 'pipep'],
  ['previous-layout', 'prevl'],
  ['previous-window', 'prev'],
  ['refresh-client', 'refresh'],
  ['rename-session', 'rename'],
  ['rename-window', 'renamew'],
  ['resize-pane', 'resizep'],
  ['resize-window', 'resizew'],
  ['respawn-pane', 'respawnp'],
  ['respawn-window', 'respawnw'],
  ['rotate-window', 'rotatew'],
  ['run-shell', 'run'],
  ['save-buffer', 'saveb'],
  ['select-layout', 'selectl'],
  ['select-pane', 'selectp'],
  ['select-window', 'selectw'],
  ['send-keys', 'send'],
  ['send-prefix'],
  ['server-access'],
  ['set-buffer', 'setb'],
  ['set-environment', 'setenv'],
  ['set-hook'],
  ['set-option', 'set'],
  ['set-window-option', 'setw'],
  ['show-buffer', 'showb'],
  ['show-environment', 'showenv'],
  ['show-hooks'],
  ['show-messages', 'showmsgs'],
  ['show-options', 'show'],
  ['show-prompt-history', 'showphist'],
  ['show-window-options', 'showw'],
  ['source-file', 'source'],
  ['split-window', 'splitw'],
  ['start-server', 'start'],
  ['suspend-client', 'suspendc'],
  ['swap-pane', 'swapp'],
  ['swap-window', 'swapw'],
  ['switch-client', 'switchc'],
  ['unbind-key', 'unbind'],
  ['unlink-window', 'unlinkw'],
  ['wait-for', 'wait'],
];

/** The command tmux runs for a typed name, or undefined when it is unknown or ambiguous. */
export function resolveTmuxCommand(word) {
  const byName = TMUX_COMMANDS.find(([name, alias]) => name === word || alias === word);
  if (byName) return byName[0];
  const prefixed = TMUX_COMMANDS.filter(([name]) => name.startsWith(word));
  return word && prefixed.length === 1 ? prefixed[0][0] : undefined;
}

const SHELLS = new Set(['sh', 'bash', 'zsh', 'dash', 'ksh']);
// Words that run another command; their own options and arguments are skipped to reach it.
const WRAPPERS = new Set([
  'env',
  'command',
  'builtin',
  'exec',
  'nohup',
  'time',
  'nice',
  'sudo',
  'doas',
  'stdbuf',
  'timeout',
  'setsid',
  'xargs',
  'caffeinate',
  'if',
  'then',
  'do',
  'else',
  'elif',
  'while',
  'until',
  '!',
  '{',
  '}',
]);
const ASSIGNMENT = /^[A-Za-z_][A-Za-z0-9_]*=/;

/**
 * Every simple command in a shell command line, nested `$(...)`, backticks and here-document
 * bodies included, as lists of words with their quotes removed. Not a shell: it follows quotes,
 * escapes, separators, comments and substitutions well enough to tell a command from text.
 */
export function simpleCommands(source) {
  const commands = [];
  let position = 0;

  /** One command list up to `closer` (or the end); nested commands land in `commands` too. */
  function list(closer) {
    let words = [];
    let word;
    let depth = 0;
    const heredocs = [];
    const end = () => {
      if (word) words.push(word.value);
      word = undefined;
      if (words.length) commands.push(words);
      words = [];
    };
    const push = () => {
      if (word) words.push(word.value);
      word = undefined;
    };
    const add = (text) => {
      word = { value: (word?.value ?? '') + text };
    };
    while (position < source.length) {
      const c = source[position];
      if (c === closer && depth === 0) {
        position += 1;
        end();
        return;
      }
      if (c === '\n') {
        position += 1;
        end();
        for (const { delimiter, strip } of heredocs.splice(0)) {
          while (position < source.length) {
            const next = source.indexOf('\n', position);
            const line = source.slice(position, next < 0 ? source.length : next);
            position = next < 0 ? source.length : next + 1;
            if ((strip ? line.replace(/^\t+/, '') : line) === delimiter) break;
          }
        }
        continue;
      }
      if (c === ' ' || c === '\t') {
        position += 1;
        push();
        continue;
      }
      if (c === '#' && !word) {
        const next = source.indexOf('\n', position);
        position = next < 0 ? source.length : next;
        continue;
      }
      if (c === '\\') {
        if (source[position + 1] === '\n') position += 2;
        else {
          add(source[position + 1] ?? '');
          position += 2;
        }
        continue;
      }
      if (c === "'") {
        const next = source.indexOf("'", position + 1);
        const stop = next < 0 ? source.length : next;
        add(source.slice(position + 1, stop));
        position = stop + 1;
        continue;
      }
      if (c === '"') {
        position += 1;
        let text = '';
        while (position < source.length && source[position] !== '"') {
          const d = source[position];
          if (d === '\\') {
            const escaped = source[position + 1] ?? '';
            text += '"$`\\\n'.includes(escaped)
              ? escaped === '\n'
                ? ''
                : escaped
              : `\\${escaped}`;
            position += 2;
          } else if (d === '$' && source[position + 1] === '(') {
            position += 2;
            list(')');
            text += '$(…)';
          } else if (d === '`') {
            position += 1;
            list('`');
            text += '`…`';
          } else {
            text += d;
            position += 1;
          }
        }
        position += 1;
        add(text);
        continue;
      }
      if (c === '$' && source[position + 1] === '(') {
        if (source[position + 2] === '(') {
          // Arithmetic: skip to the closing `))`.
          const next = source.indexOf('))', position);
          add('$((…))');
          position = next < 0 ? source.length : next + 2;
        } else {
          position += 2;
          list(')');
          add('$(…)');
        }
        continue;
      }
      if (c === '`') {
        position += 1;
        list('`');
        add('`…`');
        continue;
      }
      if (c === '<' && source[position + 1] === '<' && source[position + 2] !== '<') {
        position += 2;
        const strip = source[position] === '-';
        if (strip) position += 1;
        while (source[position] === ' ' || source[position] === '\t') position += 1;
        let delimiter = '';
        while (position < source.length && !' \t\n;&|()<>'.includes(source[position])) {
          const d = source[position];
          if (d === "'" || d === '"') {
            const next = source.indexOf(d, position + 1);
            const stop = next < 0 ? source.length : next;
            delimiter += source.slice(position + 1, stop);
            position = stop + 1;
          } else if (d === '\\') {
            delimiter += source[position + 1] ?? '';
            position += 2;
          } else {
            delimiter += d;
            position += 1;
          }
        }
        heredocs.push({ delimiter, strip });
        continue;
      }
      if (c === '(' && closer === ')') {
        depth += 1;
        end();
        position += 1;
        continue;
      }
      if (c === ')' && depth > 0) {
        depth -= 1;
        end();
        position += 1;
        continue;
      }
      if (';&|()'.includes(c)) {
        position += 1;
        end();
        continue;
      }
      add(c);
      position += 1;
    }
    end();
  }

  list(undefined);
  return commands;
}

const basename = (word) => word.slice(word.lastIndexOf('/') + 1);

/** The uid-specific directories that hold the host's default tmux socket. */
function hostSocketDirectories(env, uid) {
  const directories = [`/tmp/tmux-${uid}`, `/private/tmp/tmux-${uid}`];
  if (env.TMUX_TMPDIR) directories.push(path.join(env.TMUX_TMPDIR, `tmux-${uid}`));
  const current = env.TMUX?.split(',')[0];
  if (current) directories.push(path.dirname(current));
  return directories;
}

/** Whether a literal -S path names the host server; a path built by the shell cannot be judged. */
function isHostSocket(socket, env, uid) {
  if (/[$`*?]/.test(socket)) return false;
  const resolved = path.resolve(socket);
  const current = env.TMUX?.split(',')[0];
  return (
    (current !== undefined && current !== '' && resolved === path.resolve(current)) ||
    hostSocketDirectories(env, uid).some(
      (directory) => resolved === directory || resolved.startsWith(`${directory}/`)
    )
  );
}

/** The flags of a tmux command up to its first positional word, with the value of each -t. */
function commandFlags(args) {
  const flags = new Set();
  const targets = [];
  let index = 0;
  for (; index < args.length && /^-[A-Za-z]/.test(args[index]); index += 1) {
    const cluster = args[index].slice(1);
    for (let at = 0; at < cluster.length; at += 1) {
      flags.add(cluster[at]);
      if (cluster[at] === 't') {
        const attached = cluster.slice(at + 1);
        targets.push(attached || args[(index += 1)] || '');
        break;
      }
    }
  }
  return { flags, targets };
}

// A lead may retire a member window it opened by exact name; `=` turns off prefix matching.
const EXACT_WINDOW = /^=[^:]+:=.+$/;

/** What one tmux command (after the global options) does to the host server, if anything. */
function hostHarm(command, args) {
  const name = resolveTmuxCommand(command);
  const { flags, targets } = commandFlags(args);
  switch (name) {
    case 'kill-server':
    case 'kill-session':
      return `tmux ${name} stops the host server's agents`;
    case 'kill-pane':
      return 'tmux kill-pane closes a pane on the host server';
    case 'kill-window':
      return !flags.has('a') && targets.length === 1 && EXACT_WINDOW.test(targets[0])
        ? undefined
        : 'tmux kill-window closes a window on the host server unless it names =<session>:=<name>';
    case 'set-option':
    case 'set-window-option':
      return flags.has('g') || flags.has('s')
        ? `tmux ${name} -g/-s changes a host server option`
        : undefined;
    case 'set-hook':
      return 'tmux set-hook changes a host server hook';
    default:
      return undefined;
  }
}

/** The tmux invocations in one simple command, or the commands it runs through a shell. */
function invocations(words, found) {
  let at = 0;
  while (at < words.length && ASSIGNMENT.test(words[at])) at += 1;
  if (at === words.length) return;
  if (WRAPPERS.has(basename(words[at]))) {
    const next = words.findIndex(
      (word, index) =>
        index > at && (basename(word) === 'tmux' || SHELLS.has(basename(word)) || word === 'eval')
    );
    if (next < 0) return;
    at = next;
  }
  const program = basename(words[at]);
  if (program === 'tmux') found.tmux.push(words.slice(at + 1));
  else if (program === 'eval') found.scripts.push(words.slice(at + 1).join(' '));
  else if (SHELLS.has(program)) {
    for (let index = at + 1; index < words.length; index += 1) {
      if (/^-[A-Za-z]*c[A-Za-z]*$/.test(words[index]) && words[index + 1] !== undefined) {
        found.scripts.push(words[index + 1]);
        break;
      }
    }
  }
}

/** Splits tmux's arguments into its commands the way tmux does: a `;` argument ends one. */
function tmuxCommands(args) {
  const commands = [[]];
  for (const arg of args) {
    if (arg.endsWith('\\;')) commands.at(-1).push(`${arg.slice(0, -2)};`);
    else if (arg.endsWith(';')) {
      if (arg.length > 1) commands.at(-1).push(arg.slice(0, -1));
      commands.push([]);
    } else commands.at(-1).push(arg);
  }
  return commands.filter((command) => command.length > 0);
}

/** Global options of a tmux invocation: the -S socket, whether -L names a server, the rest. */
function globalOptions(args) {
  let socket;
  let index = 0;
  for (; index < args.length && /^-[A-Za-z0-9]/.test(args[index]); index += 1) {
    const cluster = args[index].slice(1);
    for (let at = 0; at < cluster.length; at += 1) {
      if ('SLcfT'.includes(cluster[at])) {
        const value = cluster.slice(at + 1) || args[(index += 1)] || '';
        if (cluster[at] === 'S') socket = value;
        break;
      }
    }
  }
  return { socket, rest: args.slice(index) };
}

/**
 * The refusals for a Bash command line: one per tmux command that would harm the host server.
 * A command with an explicit -S socket that is not a host socket is private and never refused.
 */
export function hostTmuxViolations(commandLine, env = process.env, uid = process.getuid?.()) {
  const violations = [];
  const pending = [commandLine];
  for (let depth = 0; pending.length > 0 && depth < 20; depth += 1) {
    const found = { tmux: [], scripts: [] };
    for (const script of pending.splice(0)) {
      for (const words of simpleCommands(script)) invocations(words, found);
    }
    pending.push(...found.scripts);
    for (const args of found.tmux) {
      const { socket, rest } = globalOptions(args);
      if (socket !== undefined && !isHostSocket(socket, env, uid)) continue;
      for (const command of tmuxCommands(rest)) {
        const harm = hostHarm(command[0], command.slice(1));
        if (harm) {
          const shown = `tmux ${command.join(' ')}`;
          violations.push({
            command: shown.length > 100 ? `${shown.slice(0, 97)}...` : shown,
            reason: harm,
          });
        }
      }
    }
  }
  return violations;
}

export function refusal(violations) {
  return [
    'Blocked by the repository hook for Host tmux safety (AGENTS.md, #4189): this command would reach the host tmux server.',
    ...violations.map(({ command, reason }) => `  ${command}\n    ${reason}`),
    "The host server runs every agent on this machine. For a scratch server run `scripts/private-tmux.sh <scratch-dir> <tmux command>`; on the host use `tmt` commands and read or type into an explicit pane. A lead may retire its member window only by exact name: kill-window -t '=<session>:=<name>'.",
    'If a command reached the host server by mistake, stop and tell your lead.',
  ].join('\n');
}

/** The hook payload to a decision: undefined to allow, else the message that blocks. */
export function decide(payload, env = process.env, uid = process.getuid?.()) {
  const command = payload?.tool_name === 'Bash' ? payload.tool_input?.command : undefined;
  if (typeof command !== 'string') return undefined;
  const violations = hostTmuxViolations(command, env, uid);
  return violations.length > 0 ? refusal(violations) : undefined;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const message = decide(JSON.parse(readFileSync(0, 'utf8')));
    if (message) {
      process.stderr.write(`${message}\n`);
      process.exitCode = 2;
    }
  } catch (error) {
    // A broken guard must not block every command: report it and let the command run.
    process.stderr.write(`host-tmux-guard could not check this command: ${error.message}\n`);
    process.exitCode = 1;
  }
}
