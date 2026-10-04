#!/usr/bin/env node
// Deterministic recipient agent for the Colab acceptance, run in a tmux pane.
// For each queued-request wake it pulls the real request with `tmt x show`,
// appends one `received` row to a durable JSONL counter BEFORE doing anything
// else, then answers through the real `tmt reply`. The scenario counts work
// from these rows, never from terminal echo, so a duplicate wake is visible as
// a second `received` row for the same request.
//
// Usage: recipient.mjs <tmt-executable> <log-path> [gate-dir]
// A gate directory parks the reply until `<gate>/<requestId>.release` exists.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import readline from 'node:readline';

const [tmt, logPath, gateDirectory] = process.argv.slice(2);
if (!tmt || !logPath) {
  console.error('usage: recipient.mjs <tmt> <log> [gate-dir]');
  process.exit(2);
}
const log = (row) => fs.appendFileSync(logPath, `${JSON.stringify(row)}\n`);
const WAKE = /^▚ ◆ [^\r\n]+ · tmt x show (req_[0-9a-f-]+) --incoming --identity (\S+) --json$/;

function run(args, stdin) {
  return new Promise((resolve) => {
    const child = spawn(tmt, args, { stdio: ['pipe', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => (stdout += chunk));
    child.stderr.on('data', (chunk) => (stderr += chunk));
    child.stdin.on('error', () => {});
    child.stdin.end(stdin ?? '');
    child.once('close', (code) => resolve({ code, stdout, stderr }));
  });
}

// The show document is the only source of the message and reply receipt.
function find(value, key) {
  if (value === null || typeof value !== 'object') return undefined;
  if (typeof value[key] === 'string') return value[key];
  for (const child of Object.values(value)) {
    const found = find(child, key);
    if (found !== undefined) return found;
  }
  return undefined;
}

// One counted delivery. The `received` row is durable before any reply work.
async function answer(source, requestId, identityId, receipt, message, extra = {}) {
  const digest = createHash('sha256').update(message).digest('hex');
  log({
    event: 'received',
    source,
    requestId,
    identityId,
    message,
    messageDigest: digest,
    ...extra,
  });
  if (!receipt) return log({ event: 'failure', stage: 'receipt', requestId });
  if (gateDirectory) {
    while (!fs.existsSync(`${gateDirectory}/${requestId}.release`))
      await new Promise((resolve) => setTimeout(resolve, 20));
  }
  const body = `ask-reply:${digest.slice(0, 16)}`;
  const reply = await run(['reply', requestId, '--receipt', receipt, '--stdin', '--json'], body);
  log({
    event: reply.code === 0 ? 'replied' : 'failure',
    stage: 'reply',
    requestId,
    body,
    code: reply.code,
    stderr: reply.stderr,
  });
}

// Remote dispatch wakes only name the request; the show document carries the
// exact delivered text and the reply receipt.
async function handleWake(requestId, identityId) {
  const shown = await run([
    'x',
    'show',
    requestId,
    '--incoming',
    '--identity',
    identityId,
    '--json',
  ]);
  let document;
  try {
    document = JSON.parse(shown.stdout);
  } catch {
    return log({
      event: 'failure',
      stage: 'show',
      requestId,
      code: shown.code,
      stderr: shown.stderr,
    });
  }
  const message = find(document, 'message') ?? find(document, 'text') ?? '';
  await answer('wake', requestId, identityId, find(document, 'receipt'), message, { document });
}

log({ event: 'ready', pid: process.pid });
// Deliveries are serialized so the counter order is the delivery order.
let chain = Promise.resolve();
let pending = [];
let frame = false;
readline.createInterface({ input: process.stdin, terminal: false }).on('line', (line) => {
  const wake = WAKE.exec(line);
  if (wake) return void (chain = chain.then(() => handleWake(wake[1], wake[2])));
  // `tmt talk` framing: message lines, then a <tmt-reply> block holding the command.
  if (/^<tmt-reply from="[^"<]*">$/.test(line)) {
    frame = true;
    return;
  }
  if (frame) {
    const command = /^tmt reply (req_[0-9a-f-]+) --receipt (\S+) --message <text>$/.exec(line);
    if (command) {
      const message = pending.join('\n').replace(/\n+$/, '');
      pending = [];
      chain = chain.then(() => answer('talk', command[1], null, command[2], message));
    }
    if (line === '</tmt-reply>') frame = false;
    return;
  }
  log({ event: 'input', line });
  pending.push(line);
});
