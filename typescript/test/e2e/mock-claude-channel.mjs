#!/usr/bin/env node

// A stand-in for Claude Code 2.1.285 with development-channel support, for the
// claude-channel E2E scenario. It is not a provider test: it plays only the MCP
// client side Claude plays for a stdio channel server, and records what reaches
// it as a channel notification versus what is typed or pasted into its terminal.
//
// Environment (all set by the scenario):
//   MOCK_CHANNEL_LOG       JSON-lines event log (required)
//   MOCK_VERSION           the `--version` line (default `2.1.285 (Claude Code)`)
//   MOCK_HANDSHAKE         complete (default) | delay | never
//   MOCK_HANDSHAKE_DELAY_MS  for `delay`
//   MOCK_AUTOREPLY=1       answer a channel request with `tmt reply`
//   MOCK_RESULT_ON_HINT=1  on a reply hint, read the durable response row at that
//                          moment (read-only SQL on MOCK_DB, no TMT command)
//   MOCK_REQUEST_ON_WAKE=1 on a request wake, read the durable request row at that
//                          moment (read-only SQL on MOCK_DB, no TMT command)
//   MOCK_DB                the TMT database, for MOCK_RESULT_ON_HINT and MOCK_REQUEST_ON_WAKE
//   MOCK_TURN_TRANSCRIPT   private telemetry fixture; prompt follows SessionStart,
//                          then `<log>.stop-turn` explicitly releases one Stop
// Control files next to the log: `<log>.kill-server` kills the channel server
// (a crash while the session lives), `<log>.quit` ends the mock cleanly.

import Database from 'better-sqlite3';
import fs from 'node:fs';
import { execFile, spawn } from 'node:child_process';
import readline from 'node:readline';
import net from 'node:net';
import { resolveCliExecutables } from '../support/cli-executable.mjs';

const args = process.argv.slice(2);
const version = process.env.MOCK_VERSION ?? '2.1.285 (Claude Code)';
if (args.includes('--version')) {
  process.stdout.write(`${version}\n`);
  process.exit(0);
}

const logPath = process.env.MOCK_CHANNEL_LOG;
if (!logPath) throw new Error('MOCK_CHANNEL_LOG is required.');
const log = (event) =>
  fs.appendFileSync(logPath, `${JSON.stringify({ ...event, pid: process.pid })}\n`);
const handshake = process.env.MOCK_HANDSHAKE ?? 'complete';
const { peer } = resolveCliExecutables();
const peerCompletions = new Set();
let stopping = false;

// A mock-owned hook/reply can write fixture state after its caller disappears.
// Retain close receipts until graceful shutdown has settled every peer.
function execPeer(args, callback) {
  const child = execFile(peer.executable, [...peer.args, ...args], { env: process.env }, callback);
  const closed = new Promise((resolve) => child.once('close', resolve));
  peerCompletions.add(closed);
  void closed.then(() => peerCompletions.delete(closed));
  return child;
}

function emitHook(payload, event) {
  if (stopping) return Promise.reject(new Error('fixture shutdown already started'));
  if (process.env.MOCK_LAUNCH_HOOKS === '1') return emitInstalledHooks(payload, event);
  return new Promise((resolve, reject) => {
    const child = execPeer(['__hook', 'claude'], (error, stdout, stderr) => {
      if (error || stderr) reject(error ?? new Error(stderr));
      else {
        log({ event, stdout });
        resolve();
      }
    });
    child.stdin.end(JSON.stringify(payload));
  });
}

// Linux-only negative handoff fixture. An empty pipe with exactly 4096 bytes
// capacity is left unread until the real generated hook exits. A larger JSON
// response therefore publishes a prefix, exhausts its own deadline, and leaves
// the sealed attempt uncertain/claimed. The fixture owns and reaps its child;
// it never substitutes a settlement or changes the product work budget.
const partialDigestHandoff = `
import fcntl, json, os, signal, subprocess, sys
payload = sys.stdin.buffer.read()
read, write = os.pipe()
assert fcntl.fcntl(write, fcntl.F_SETPIPE_SZ, 4096) == 4096
child = None
try:
    child = subprocess.Popen(['/bin/sh', '-c', 'exec ' + sys.argv[1]], stdin=subprocess.PIPE,
                             stdout=write, stderr=subprocess.PIPE, start_new_session=True)
    os.close(write)
    write = None
    _, error = child.communicate(payload, timeout=5)
    if error or child.returncode != 0:
        raise RuntimeError('generated Digest hook failed')
    chunks = []
    while True:
        chunk = os.read(read, 4096)
        if not chunk:
            break
        chunks.append(chunk)
    published = b''.join(chunks)
    assert len(published) == 4096, 'fixture requires a real partial publication'
    try:
        json.loads(published)
    except (ValueError, UnicodeDecodeError):
        pass
    else:
        raise AssertionError('fixture received complete JSON, not a partial handoff')
    print(json.dumps({'partialBytes':len(published), 'code':child.returncode}))
finally:
    if child is not None and child.poll() is None:
        os.killpg(child.pid, signal.SIGKILL)
        child.wait()
    if write is not None:
        os.close(write)
    os.close(read)
`;

// Digest scenarios execute the actual per-launch configuration. Other channel
// scenarios deliberately retain their observation-only fake provider contract.
async function emitInstalledHooks(
  payload,
  event,
  rejectDigestOutput = false,
  partialOutput = false
) {
  const index = args.indexOf('--settings');
  if (index < 0) throw new Error('tmt run did not install launch settings');
  const globalFile = `${process.env.CLAUDE_CONFIG_DIR ?? `${process.env.HOME}/.claude`}/settings.json`;
  const global = fs.existsSync(globalFile) ? JSON.parse(fs.readFileSync(globalFile, 'utf8')) : {};
  const inline = JSON.parse(args[index + 1]);
  const commands = new Set(
    [global, inline].flatMap((settings) =>
      (settings.hooks?.[payload.hook_event_name] ?? []).flatMap((entry) =>
        entry.hooks.map((hook) => hook.command)
      )
    )
  );
  const outputs = await Promise.all(
    [...commands].map(
      (command) =>
        new Promise((resolve, reject) => {
          const effectiveCommand =
            rejectDigestOutput && command.includes(' __digest-hook ')
              ? `${command} >/dev/full`
              : command;
          const partial = partialOutput && command.includes(' __digest-hook ');
          const child = execFile(
            partial ? 'python3' : '/bin/sh',
            partial ? ['-c', partialDigestHandoff, command] : ['-c', effectiveCommand],
            { env: process.env },
            (error, stdout, stderr) => {
              if (error || stderr) reject(error ?? new Error(stderr));
              else {
                log({
                  event: 'installed-hook',
                  hookEvent: payload.hook_event_name,
                  command,
                  stdout,
                });
                if (partial) {
                  log({ event: 'digest-partial-handoff', ...JSON.parse(stdout) });
                  resolve('');
                } else resolve(stdout);
              }
            }
          );
          const closed = new Promise((resolve) => child.once('close', resolve));
          peerCompletions.add(closed);
          void closed.then(() => peerCompletions.delete(closed));
          child.stdin.end(JSON.stringify(payload));
        })
    )
  );
  const continuations = outputs
    .filter(Boolean)
    .map((text) => JSON.parse(text))
    .filter((value) => value.decision === 'block');
  log({ event, stdout: outputs.join('') });
  for (const continuation of continuations) {
    log({ event: 'digest-continuation', reason: continuation.reason });
    // The provider's causal continuation invokes Stop again with its loop guard.
    // That second event must not claim a later arrival or repeat the batch.
    await emitInstalledHooks({ ...payload, stop_hook_active: true }, 'recursive-stop');
  }
}

let turnReady = false;
let turnSubmitted = false;
let digestStepRunning = false;

let server;
let serverClosed;
const configIndex = args.indexOf('--mcp-config');
if (configIndex >= 0) {
  const flagIndex = args.indexOf('--dangerously-load-development-channels');
  const config = JSON.parse(args[configIndex + 1]);
  const { command, args: serverArgs } = config.mcpServers.tmt;
  log({
    event: 'launch',
    channel: flagIndex >= 0 ? args[flagIndex + 1] : null,
    strictMcpConfig: args.includes('--strict-mcp-config'),
    serverArgs,
  });
  if (process.env.TMT_TEST_CLAUDE_MCP_SOCKET) {
    // The native fixture owns the actual MCP child, matching the admitted
    // provider incarnation. This socket forwards only its real stdio bytes.
    const socket = net.createConnection(process.env.TMT_TEST_CLAUDE_MCP_SOCKET);
    server = { stdin: socket, stdout: socket };
    serverClosed = new Promise((resolve) => socket.once('close', resolve));
  } else {
    server = spawn(command, serverArgs, { stdio: ['pipe', 'pipe', 'inherit'], env: process.env });
    server.on('exit', (code, signal) => log({ event: 'server-exit', code, signal }));
    serverClosed = new Promise((resolve) => server.once('close', resolve));
  }
  const send = (message) => server.stdin.write(`${JSON.stringify(message)}\n`);
  const announce = () => {
    send({ jsonrpc: '2.0', method: 'notifications/initialized' });
    log({ event: 'initialized-sent' });
  };
  readline.createInterface({ input: server.stdout }).on('line', (line) => {
    const message = JSON.parse(line);
    if (message.id === 1) {
      log({ event: 'initialize-result', capabilities: message.result.capabilities });
      if (handshake === 'complete') announce();
      if (handshake === 'delay') {
        setTimeout(announce, Number(process.env.MOCK_HANDSHAKE_DELAY_MS ?? 1000));
      }
      return;
    }
    if (message.method === 'notifications/claude/channel') {
      const content = message.params.content;
      log({ event: 'channel', content });
      const reply = /tmt reply (\S+) --receipt (\S+) --message <text>/.exec(content);
      if (process.env.MOCK_AUTOREPLY === '1' && reply) {
        execPeer(
          ['reply', reply[1], '--receipt', reply[2], '--message', 'channel-ok', '--json'],
          (error) => log({ event: 'reply', ok: error === null })
        );
      }
      const wake = /\breq_[0-9a-f-]{36}\b/.exec(content);
      if (process.env.MOCK_REQUEST_ON_WAKE === '1' && wake) {
        // A request is committed to the recipient's inbox before it is announced.
        const database = new Database(process.env.MOCK_DB, { readonly: true });
        try {
          const row = database
            .prepare('SELECT message_text FROM request_attempts WHERE request_id = ?')
            .get(wake[0]);
          log({
            event: 'wake-request',
            requestId: wake[0],
            messageText: row?.message_text ?? null,
          });
        } finally {
          database.close();
        }
      }
      // The result command ends the notice's first line; an inlined reply body follows it.
      const hint = /\btmt result (\S+)$/.exec(content.split('\n')[0]);
      if (process.env.MOCK_RESULT_ON_HINT === '1' && hint) {
        // The response is committed before the hint is sent, so the row exists by
        // the time the hint first reaches the provider. Read-only, no side effects.
        const database = new Database(process.env.MOCK_DB, { readonly: true });
        try {
          // Correlate a short result operand independently against request IDs,
          // with exact IDs taking precedence. A timeout also carries a result
          // command; it must not manufacture response evidence before a final.
          const ids = database
            .prepare(
              'SELECT request_id FROM request_attempts WHERE request_id = ? OR substr(request_id, 1, 12) = ?'
            )
            .all(hint[1], /^[0-9a-f]{8}$/.test(hint[1]) ? `req_${hint[1]}` : '');
          const exact = ids.find((row) => row.request_id === hint[1]);
          const selected = exact ?? (ids.length === 1 ? ids[0] : undefined);
          if (selected) {
            const row = database
              .prepare('SELECT body FROM request_responses WHERE request_id = ?')
              .get(selected.request_id);
            if (row)
              log({ event: 'hint-response', requestId: selected.request_id, body: row.body });
          }
        } finally {
          database.close();
        }
      }
    }
  });
  send({
    jsonrpc: '2.0',
    id: 1,
    method: 'initialize',
    params: {
      protocolVersion: '2025-11-25',
      capabilities: {},
      clientInfo: { name: 'claude-code', version: version.split(' ')[0] },
    },
  });
} else {
  log({ event: 'launch', channel: null });
}

// What is typed or pasted into the terminal is a paste, whatever else happens.
readline
  .createInterface({ input: process.stdin })
  .on('line', (line) => log({ event: 'paste', line }));

const control = setInterval(() => {
  if (fs.existsSync(`${logPath}.quit`)) stopping = true;
  if (fs.existsSync(`${logPath}.kill-server`)) {
    fs.rmSync(`${logPath}.kill-server`);
    server?.kill('SIGKILL');
  }
  if (
    !stopping &&
    turnReady &&
    !digestStepRunning &&
    process.env.MOCK_LAUNCH_HOOKS === '1' &&
    fs.existsSync(`${logPath}.digest-step`)
  ) {
    const step = JSON.parse(fs.readFileSync(`${logPath}.digest-step`, 'utf8'));
    fs.rmSync(`${logPath}.digest-step`);
    digestStepRunning = true;
    void emitInstalledHooks(
      {
        hook_event_name: 'Stop',
        session_id: step.session ?? process.env.MOCK_SESSION_ID,
        stop_hook_active: step.active ?? false,
      },
      step.event,
      step.rejectDigestOutput === true,
      step.partialDigestOutput === true
    )
      .catch((error) => {
        log({ event: 'hook-error', message: error.message });
        process.exitCode = 1;
      })
      .finally(() => {
        digestStepRunning = false;
      });
  }
  if (!stopping && turnReady && !turnSubmitted && fs.existsSync(`${logPath}.stop-turn`)) {
    turnSubmitted = true;
    void emitHook(
      {
        hook_event_name: 'Stop',
        session_id: process.env.MOCK_SESSION_ID,
        transcript_path: process.env.MOCK_TURN_TRANSCRIPT,
        ...(process.env.MOCK_LAUNCH_HOOKS === '1' ? { stop_hook_active: false } : {}),
      },
      'turn-recorded'
    ).catch((error) => {
      log({ event: 'hook-error', message: error.message });
      process.exitCode = 1;
    });
  }
  if (fs.existsSync(`${logPath}.quit`)) {
    clearInterval(control);
    log({ event: 'shutdown-start' });
    server?.stdin.end();
    // No new channel callbacks can start peers after the server's streams close.
    void Promise.resolve(serverClosed)
      .then(() => Promise.all(peerCompletions))
      .then(() => {
        log({ event: 'stopped' });
        process.exit(process.exitCode ?? 0);
      });
  }
}, 50);
log({ event: 'started', args });

// Opt-in lifecycle scenario: the native parent is the real observed runtime.
// Wait for its admission, then emit the provider's start payload.
if (process.env.MOCK_SESSION_ID) {
  const deadline = Date.now() + 10_000;
  const attach = async () => {
    for (;;) {
      const database = new Database(process.env.MOCK_DB, { readonly: true });
      const bound = database
        .prepare('SELECT runtime_state FROM bindings WHERE runtime_pid = ?')
        .get(process.ppid);
      database.close();
      if (bound?.runtime_state === 'running') break;
      if (Date.now() >= deadline) throw new Error('native fixture runtime was not admitted');
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    const payload = {
      hook_event_name: 'SessionStart',
      source: args.includes('--resume') ? 'resume' : 'startup',
      session_id: process.env.MOCK_SESSION_ID,
      model: 'model-a',
      ...(process.env.MOCK_TURN_TRANSCRIPT
        ? { transcript_path: process.env.MOCK_TURN_TRANSCRIPT }
        : {}),
    };
    await emitHook(payload, 'hook-recorded');
    if (process.env.MOCK_TURN_TRANSCRIPT || process.env.MOCK_LAUNCH_HOOKS === '1') {
      await emitHook({ ...payload, hook_event_name: 'UserPromptSubmit' }, 'prompt-recorded');
      turnReady = true;
    }
  };
  attach().catch((error) => {
    log({ event: 'hook-error', message: error.message });
    process.exitCode = 1;
  });
}
