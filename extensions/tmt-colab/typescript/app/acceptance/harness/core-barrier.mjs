#!/usr/bin/env node
// TMT_EXECUTABLE for the real tmt-remote and tmt-colab children. It forwards
// every invocation to the real core binary byte for byte, records each launch,
// and can park one dispatch.create at a deterministic barrier so the scenario
// can kill and restart a process there. It never supplies a result of its own.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const realCore = process.env.TMT_ACCEPTANCE_REAL_TMT;
const directory = process.env.TMT_ACCEPTANCE_BARRIER_DIR;
if (!realCore || !directory) {
  console.error('core-barrier needs TMT_ACCEPTANCE_REAL_TMT and TMT_ACCEPTANCE_BARRIER_DIR');
  process.exit(2);
}
const argv = process.argv.slice(2);
const started = performance.now();
const input = fs.readFileSync(0);
let operation = null;
let operationId = null;
if (argv[0] === 'api') {
  try {
    const request = JSON.parse(input.toString('utf8'));
    operation = request.operation ?? null;
    operationId = request.input?.operationId ?? null;
  } catch {
    // Not JSON: forwarded unchanged and recorded without an operation.
  }
}
fs.appendFileSync(
  path.join(directory, 'calls.jsonl'),
  `${JSON.stringify({ operation, operationId, pid: process.pid })}\n`,
);

let barrier = null;
try {
  barrier = JSON.parse(fs.readFileSync(path.join(directory, 'barrier.json'), 'utf8'));
} catch {
  // No barrier armed.
}
const gated =
  operation === 'dispatch.create' &&
  barrier !== null &&
  (barrier.operationId === null || barrier.operationId === operationId);

async function park(phase) {
  if (!gated || barrier.phase !== phase) return;
  fs.writeFileSync(
    path.join(directory, 'entered.json'),
    JSON.stringify({ phase, operationId, pid: process.pid }),
  );
  const release = path.join(directory, 'release');
  const deadline = Date.now() + 60_000;
  while (!fs.existsSync(release)) {
    if (Date.now() >= deadline) throw new Error('Barrier was never released');
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

await park('before');
const child = spawn(realCore, argv, { stdio: ['pipe', gated ? 'pipe' : 'inherit', 'inherit'] });
child.stdin.on('error', () => {});
child.stdin.end(input);
const output = [];
if (gated) child.stdout.on('data', (chunk) => output.push(chunk));
const exit = await new Promise((resolve, reject) => {
  child.once('error', reject);
  child.once('close', (code, signal) => resolve({ code, signal }));
});
if (argv.join(' ') === 'remote status --json') {
  fs.appendFileSync(
    path.join(directory, 'discovery.jsonl'),
    `${JSON.stringify({ elapsedMs: performance.now() - started, ...exit })}\n`,
  );
}
if (gated) {
  // The core already acted; hold only the response so a crash here proves
  // recovery without a second wake.
  await park('after');
  process.stdout.write(Buffer.concat(output));
}
if (exit.signal) process.kill(process.pid, exit.signal);
else process.exitCode = exit.code ?? 1;
