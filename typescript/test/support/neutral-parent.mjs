// Test infrastructure only: start the selected executable beneath PID 1,
// as its own process-group leader, after the harness acknowledges ownership.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const [mode, ...args] = process.argv.slice(2);
const script = import.meta.filename;
const nodeArgs = ['--disable-warning=ExperimentalWarning', script];

function report(value) {
  fs.writeSync(3, JSON.stringify(value) + '\n');
}

function reportError(error) {
  report({ error: { message: error.message, code: error.code ?? 'NEUTRAL_PARENT_FAILED' } });
  process.exit(1);
}

function executablePath(executable) {
  const candidates = executable.includes('/')
    ? [executable]
    : (process.env.PATH ?? '/usr/bin:/bin')
        .split(path.delimiter)
        .map((directory) => path.join(directory || '.', executable));
  let denied;
  for (const candidate of candidates) {
    try {
      // Node 22 aborts on a failed execve instead of throwing an errno. Fixture
      // publication owns stable files; check access immediately before exec.
      fs.accessSync(candidate, fs.constants.X_OK);
      if (!fs.statSync(candidate).isFile())
        throw Object.assign(new Error(`Not an executable file: ${candidate}`), { code: 'EACCES' });
      return candidate;
    } catch (error) {
      if (executable.includes('/')) throw error;
      if (error.code === 'EACCES') denied ??= error;
      else if (error.code !== 'ENOENT' && error.code !== 'ENOTDIR') throw error;
    }
  }
  throw (
    denied ??
    Object.assign(new Error(`Executable not found on PATH: ${executable}`), { code: 'ENOENT' })
  );
}

if (mode === 'detach') {
  const supervisor = spawn(
    process.execPath,
    [...nodeArgs, 'supervise', String(process.pid), ...args],
    {
      stdio: ['inherit', 'inherit', 'inherit', 3, 4],
    }
  );
  supervisor.once('error', reportError);
  supervisor.once('spawn', () => process.exit(0));
} else if (mode === 'supervise') {
  const [parent, ...command] = args;
  const deadline = performance.now() + 1000;
  // Reparenting, rather than setsid/detached alone, removes provider ancestry.
  // Refuse an unknown adopter, such as a runtime subreaper.
  while (process.ppid === Number(parent)) {
    if (performance.now() >= deadline) reportError(new Error('Neutral parent was not adopted.'));
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  if (process.ppid !== 1) reportError(new Error('Neutral parent requires adoption by PID 1.'));
  const child = spawn(process.execPath, [...nodeArgs, 'execute', ...command], {
    detached: true,
    stdio: ['inherit', 'inherit', 'inherit', 3, 4],
  });
  child.once('error', reportError);
  // The selected CLI leads this second owned group. Exec preserves its PID,
  // group and inherited streams; the supervisor reports its observed exit.
  child.once('exit', (status, signal) => {
    report({ status, signal });
    process.exit(0);
  });
} else if (mode === 'execute') {
  const [executable, ...argv] = args;
  report({ group: process.pid });
  const acknowledgement = fs.createReadStream('', { fd: 4, autoClose: false });
  acknowledgement.setEncoding('utf8');
  let body = '';
  const timer = setTimeout(
    () => reportError(new Error('CLI group ownership was not acknowledged.')),
    1000
  );
  acknowledgement.on('data', (chunk) => {
    body += chunk;
    if (!'ready\n'.startsWith(body))
      return reportError(new Error('Invalid CLI group acknowledgement.'));
    if (body !== 'ready\n') return;
    clearTimeout(timer);
    try {
      // execve closes non-standard descriptors, so the product never receives
      // the harness control protocol or an environment-based guard override.
      process.execve(executablePath(executable), [executable, ...argv], process.env);
    } catch (error) {
      reportError(error);
    }
  });
  acknowledgement.once('end', () => process.exit(1));
  acknowledgement.once('error', reportError);
} else {
  reportError(new Error('Unknown neutral-parent stage.'));
}
