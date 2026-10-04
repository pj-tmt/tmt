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
// Control files next to the log: `<log>.kill-server` kills the channel server
// (a crash while the session lives), `<log>.quit` ends the mock cleanly.

import Database from 'better-sqlite3';
import fs from 'node:fs';
import { execFile, spawn } from 'node:child_process';
import readline from 'node:readline';
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

// A mock-owned hook/reply can write fixture state after its caller disappears.
// Retain close receipts until graceful shutdown has settled every peer.
function execPeer(args, callback) {
  const child = execFile(peer.executable, [...peer.args, ...args], { env: process.env }, callback);
  const closed = new Promise((resolve) => child.once('close', resolve));
  peerCompletions.add(closed);
  void closed.then(() => peerCompletions.delete(closed));
  return child;
}

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
  server = spawn(command, serverArgs, { stdio: ['pipe', 'pipe', 'inherit'], env: process.env });
  server.on('exit', (code, signal) => log({ event: 'server-exit', code, signal }));
  serverClosed = new Promise((resolve) => server.once('close', resolve));
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
  if (fs.existsSync(`${logPath}.kill-server`)) {
    fs.rmSync(`${logPath}.kill-server`);
    server?.kill('SIGKILL');
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
// Wait for its admission, then emit the provider's resumed-start payload.
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
      source: 'resume',
      session_id: process.env.MOCK_SESSION_ID,
      model: 'model-a',
    };
    await new Promise((resolve, reject) => {
      const child = execPeer(['__hook', 'claude'], (error, stdout, stderr) => {
        if (error || stderr) reject(error ?? new Error(stderr));
        else {
          log({ event: 'hook-recorded', stdout });
          resolve();
        }
      });
      child.stdin.end(JSON.stringify(payload));
    });
  };
  attach().catch((error) => {
    log({ event: 'hook-error', message: error.message });
    process.exitCode = 1;
  });
}
