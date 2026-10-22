import { execFile, spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const writer = fileURLToPath(import.meta.url);

/** A write is a local file copy and an fsync; a minute is far beyond any healthy one (#4464). */
const DEFAULT_TIMEOUT_MS = 60_000;
/** The supervisor reports and kills the writer itself; this only bounds the supervisor. */
const BACKSTOP_MS = 15_000;
const EVIDENCE_COMMAND_MS = 5_000;
const EVIDENCE_LINES = 60;

/**
 * Publish fixture bytes from a process that cannot leak a writable fd to test children.
 *
 * The bytes reach the writer as a read-only file on its stdin and its output goes to files,
 * never through a pipe: a regular file always reaches EOF and has no peer end that another
 * process spawned at the same time could inherit and hold open (#4464). The executable's only
 * writable fd still lives in the writer and is closed before the atomic rename. A writer that
 * does not finish within `timeoutMs` is killed, and the error names what it was waiting on.
 * `options.writer` replaces the writer script, for tests of the stall report.
 */
export function writeExecutable(file, contents, mode = 0o755, options = {}) {
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const directory = fs.mkdtempSync(path.join(path.dirname(file), '.tmt-executable-io-'));
  try {
    const input = path.join(directory, 'input');
    fs.writeFileSync(input, contents, { mode: 0o600 });
    const bytes = fs.statSync(input).size;
    const stderrFile = path.join(directory, 'stderr');
    const descriptors = [
      fs.openSync(input, 'r'),
      fs.openSync(path.join(directory, 'stdout'), 'w', 0o600),
      fs.openSync(stderrFile, 'w', 0o600),
    ];
    let result;
    try {
      result = spawnSync(
        process.execPath,
        [
          writer,
          '--supervise',
          file,
          String(mode),
          String(timeoutMs),
          String(bytes),
          options.writer ?? writer,
        ],
        { stdio: descriptors, timeout: timeoutMs + BACKSTOP_MS, killSignal: 'SIGKILL' }
      );
    } finally {
      for (const descriptor of descriptors) fs.closeSync(descriptor);
    }
    if (result.error || result.status !== 0) {
      const reason = result.error
        ? `the supervisor could not run or finish: ${result.error.message}`
        : `the writer ended with ${result.signal ?? `status ${result.status}`}`;
      throw new Error(
        `executable-fixture could not publish ${file} (${bytes} bytes, mode ${mode.toString(8)}): ${reason}\n${fs.readFileSync(stderrFile, 'utf8')}`
      );
    }
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
}

function publish(file, contents, mode) {
  const staging = fs.mkdtempSync(path.join(path.dirname(file), '.tmt-executable-'));
  const staged = path.join(staging, 'payload');
  try {
    const fd = fs.openSync(staged, 'wx', 0o600);
    try {
      fs.writeFileSync(fd, contents);
      fs.fsyncSync(fd);
    } finally {
      fs.closeSync(fd);
    }
    fs.chmodSync(staged, mode);
    fs.renameSync(staged, file);
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
  }
}

function run(program, args) {
  return new Promise((resolve) => {
    execFile(
      program,
      args,
      { timeout: EVIDENCE_COMMAND_MS, maxBuffer: 1 << 20 },
      (error, stdout, stderr) =>
        resolve(
          error && !stdout
            ? `unavailable (${error.code ?? error.message})`
            : `${stdout}${stderr}`.split('\n').slice(0, EVIDENCE_LINES).join('\n')
        )
    );
  });
}

/** What a writer that will not finish is doing: its state, and every file and socket it holds. */
async function evidence(pid) {
  const state = await run('ps', ['-o', 'pid,ppid,stat,etime,command', '-p', String(pid)]);
  const files = await run('lsof', ['-n', '-P', '-p', String(pid)]);
  return `ps:\n${state}\nlsof (first ${EVIDENCE_LINES} lines):\n${files}\n`;
}

/**
 * Runs the writer with this process's stdio and, if it outlives `timeoutMs`, reports what it holds
 * while it is still alive and then kills it. The report is why this is a process of its own: a
 * writer stuck in a blocking read or write cannot describe itself.
 */
async function supervise(file, mode, timeoutMs, bytes, script) {
  const child = spawn(process.execPath, [script, '--write', file, String(mode)], {
    stdio: 'inherit',
  });
  let timedOut = false;
  const exited = new Promise((resolve) =>
    child.once('close', (code, signal) => resolve({ code, signal }))
  );
  const timer = setTimeout(async () => {
    timedOut = true;
    process.stderr.write(
      `executable-fixture: the writer for ${file} (${bytes} bytes, mode ${mode.toString(8)}, pid ${child.pid}) did not finish within ${timeoutMs} ms and was killed.\n${await evidence(child.pid)}`
    );
    child.kill('SIGKILL');
  }, timeoutMs);
  const { code, signal } = await exited;
  clearTimeout(timer);
  process.exitCode = timedOut ? 124 : (code ?? (signal ? 1 : 0));
}

// Synthetic shell installers use this same writer with fixture bytes on stdin.
if (process.argv[1] === writer) {
  const [, , command, file, rawMode, ...rest] = process.argv;
  const mode = Number(rawMode);
  if (!file || !Number.isInteger(mode) || mode < 0 || mode > 0o7777) {
    throw new Error('Usage: executable-fixture.mjs --write FILE MODE');
  }
  if (command === '--write') {
    publish(file, fs.readFileSync(0), mode);
  } else if (command === '--supervise') {
    await supervise(file, mode, Number(rest[0]), Number(rest[1]), rest[2]);
  } else {
    throw new Error('Usage: executable-fixture.mjs --write FILE MODE');
  }
}
