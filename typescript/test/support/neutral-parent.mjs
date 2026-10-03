// Test infrastructure only: preserve the caller-owned process group and streams,
// but start the selected executable only after its supervisor is adopted by PID 1.
import { spawn } from 'node:child_process';
import fs from 'node:fs';

const [mode, ...args] = process.argv.slice(2);
const script = import.meta.filename;

function report(value) {
  fs.writeSync(3, JSON.stringify(value));
  fs.closeSync(3);
}

function reportError(error) {
  report({ error: { message: error.message, code: error.code ?? 'NEUTRAL_PARENT_FAILED' } });
  process.exit(1);
}

if (mode === 'relay') {
  // The intermediary exits without waiting for the supervisor. Its control pipe
  // stays open in the supervisor, whose payload completion ends this relay.
  const intermediary = spawn(process.execPath, [script, 'detach', ...args], {
    stdio: ['inherit', 'inherit', 'inherit', 'pipe'],
  });
  intermediary.once('error', reportError);
  const control = intermediary.stdio[3];
  control.setEncoding('utf8');
  let body = '';
  control.on('data', (chunk) => {
    body += chunk;
    if (body.length > 16384) reportError(new Error('Neutral-parent control exceeded its bound.'));
  });
  control.once('end', () => {
    try {
      const result = JSON.parse(body);
      report(result);
      if (result.error) process.exit(1);
      if (result.signal) process.kill(process.pid, result.signal);
      else process.exit(result.status);
    } catch (error) {
      reportError(error);
    }
  });
} else if (mode === 'detach') {
  const supervisor = spawn(process.execPath, [script, 'supervise', String(process.pid), ...args], {
    stdio: ['inherit', 'inherit', 'inherit', 3],
  });
  supervisor.once('error', reportError);
  supervisor.once('spawn', () => process.exit(0));
} else if (mode === 'supervise') {
  const [parent, executable, ...argv] = args;
  const deadline = performance.now() + 1000;
  // Reparenting, rather than setsid/detached alone, removes provider ancestry.
  // Refuse another adopter (for example a runtime subreaper); never guess that
  // an unknown parent is neutral or provide a production guard override.
  while (process.ppid === Number(parent)) {
    if (performance.now() >= deadline) reportError(new Error('Neutral parent was not adopted.'));
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  if (process.ppid !== 1) reportError(new Error('Neutral parent requires adoption by PID 1.'));
  const child = spawn(executable, argv, { stdio: 'inherit' });
  child.once('error', reportError);
  // exit, not stream close: descendants may retain the inherited descriptors.
  // The outer harness stops and verifies the group after the relayed exit.
  child.once('exit', (status, signal) => {
    report({ status, signal });
    process.exit(0);
  });
} else {
  reportError(new Error('Unknown neutral-parent stage.'));
}
