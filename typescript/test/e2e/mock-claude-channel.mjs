#!/usr/bin/env node

// A stand-in for Claude Code 2.1.285 with development-channel support, for the
// claude-channel E2E scenario. It is not a provider test: it plays only the MCP
// client side Claude plays for a stdio channel server, and records what reaches
// it as a channel notification versus what is typed or pasted into its terminal.
//
// Environment (all set by the scenario):
//   MOCK_CHANNEL_LOG       JSON-lines event log (required)
//   MOCK_HANDSHAKE         complete (default) | delay | never
//   MOCK_HANDSHAKE_DELAY_MS  for `delay`
//   MOCK_AUTOREPLY=1       answer a channel request with `tmt reply`
// Control files next to the log: `<log>.kill-server` kills the channel server
// (a crash while the session lives), `<log>.quit` ends the mock cleanly.

import fs from 'node:fs';
import { execFile, spawn } from 'node:child_process';
import readline from 'node:readline';
import { resolveCliExecutables } from '../support/cli-executable.mjs';

const args = process.argv.slice(2);
if (args.includes('--version')) {
  process.stdout.write('2.1.285 (Claude Code)\n');
  process.exit(0);
}

const logPath = process.env.MOCK_CHANNEL_LOG;
if (!logPath) throw new Error('MOCK_CHANNEL_LOG is required.');
const log = (event) =>
  fs.appendFileSync(logPath, `${JSON.stringify({ ...event, pid: process.pid })}\n`);
const handshake = process.env.MOCK_HANDSHAKE ?? 'complete';
const { peer } = resolveCliExecutables();

let server;
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
        execFile(
          peer.executable,
          [
            ...peer.args,
            'reply',
            reply[1],
            '--receipt',
            reply[2],
            '--message',
            'channel-ok',
            '--json',
          ],
          { env: process.env },
          (error) => log({ event: 'reply', ok: error === null })
        );
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
      clientInfo: { name: 'claude-code', version: '2.1.285' },
    },
  });
} else {
  log({ event: 'launch', channel: null });
}

// What is typed or pasted into the terminal is a paste, whatever else happens.
readline
  .createInterface({ input: process.stdin })
  .on('line', (line) => log({ event: 'paste', line }));

setInterval(() => {
  if (fs.existsSync(`${logPath}.kill-server`)) {
    fs.rmSync(`${logPath}.kill-server`);
    server?.kill('SIGKILL');
  }
  if (fs.existsSync(`${logPath}.quit`)) {
    server?.stdin.end();
    process.exit(0);
  }
}, 50);
log({ event: 'started' });
