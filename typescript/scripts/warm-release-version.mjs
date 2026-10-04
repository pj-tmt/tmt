// Waits out the first exec of a freshly built release-version helper (#1646). A macOS runner can
// stall a new executable's first launch for over a minute; tomlCommand keeps its 60s bound for
// real work, so the stall is absorbed here once, with evidence, before injection starts.
import { spawn, spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { TOOL } from './release-version-injection.mjs';

const SAMPLE = 'name = "warm"\n';

// Processes that explain a first-launch stall: the helper itself, build tools and the macOS
// security scanners that inspect new executables.
const SUSPECTS =
  /release-version|cargo|rustc|syspolicyd|amfid|trustd|XProtect|endpointsecurityd|MRT|taskgated/;

/** Best-effort process evidence for a stalled first exec; never throws. */
export function captureStallDiagnostics(
  pid,
  { platform = process.platform, run = spawnSync } = {}
) {
  const attempt = (command, args, filter = (text) => text) => {
    const result = run(command, args, {
      encoding: 'utf8',
      timeout: 20_000,
      maxBuffer: 8 * 1024 * 1024,
    });
    const text = filter(`${result.stdout ?? ''}${result.stderr ?? ''}`.trim());
    return `$ ${command} ${args.join(' ')}\n${text.slice(0, 20_000) || String(result.error ?? 'no output')}`;
  };
  // Header, the busiest processes by CPU, then every suspect or the helper's own pid.
  const interesting = (text) => {
    const [header, ...rows] = text.split('\n');
    const own = new RegExp(`^\\s*${pid}\\s`);
    return [
      header,
      ...rows
        .filter((row, index) => index < 15 || own.test(row) || SUSPECTS.test(row))
        .map((row) => row.slice(0, 240)),
    ].join('\n');
  };
  const parts = [attempt('ps', ['-arxo', 'pid,ppid,etime,state,%cpu,command'], interesting)];
  if (platform === 'darwin') parts.push(attempt('sample', [String(pid), '2']));
  return parts.join('\n');
}

function exec(tool, { spawnProcess, onSpawn }) {
  const child = spawnProcess(tool, ['parse'], { stdio: ['pipe', 'pipe', 'pipe'] });
  onSpawn?.(child);
  let stderr = '';
  child.stderr.on('data', (chunk) => (stderr += chunk));
  child.stdout.resume();
  child.stdin.on('error', () => {});
  child.stdin.end(SAMPLE);
  const done = new Promise((resolveDone, reject) => {
    child.once('error', reject);
    child.once('close', (status, signal) => resolveDone({ status, signal, stderr }));
  });
  return { child, done };
}

/**
 * First exec: wait up to `waitMs` without killing it early, logging every `tickMs` and capturing
 * diagnostics once at `diagnoseAtMs`. Second exec: must be quick, which proves the stall was a
 * first-launch cost. Resolves with both durations in milliseconds.
 */
export async function warmReleaseVersion({
  tool = TOOL,
  waitMs = 180_000,
  tickMs = 15_000,
  diagnoseAtMs = 60_000,
  secondBoundMs = 60_000,
  log = (line) => process.stderr.write(`${line}\n`),
  diagnose = captureStallDiagnostics,
  spawnProcess = spawn,
} = {}) {
  const started = Date.now();
  const first = exec(tool, { spawnProcess });
  const seconds = () => Math.round((Date.now() - started) / 1000);
  let diagnosed = false;
  const ticker = setInterval(() => {
    log(`release-version first exec still running after ${seconds()}s`);
    if (!diagnosed && Date.now() - started >= diagnoseAtMs) {
      diagnosed = true;
      log(`release-version stall diagnostics at ${seconds()}s:\n${diagnose(first.child.pid)}`);
    }
  }, tickMs);
  let deadline;
  const outcome = await Promise.race([
    first.done,
    new Promise((resolveDeadline) => {
      deadline = setTimeout(() => resolveDeadline(null), waitMs);
    }),
  ]).finally(() => {
    clearInterval(ticker);
    clearTimeout(deadline);
  });
  if (!outcome) {
    first.child.kill('SIGKILL');
    throw new Error(`release-version first exec did not finish within ${waitMs / 1000}s.`);
  }
  const firstMs = Date.now() - started;
  if (outcome.status !== 0 || outcome.signal)
    throw new Error(`release-version first exec failed: ${outcome.stderr}`);
  log(`release-version first exec took ${(firstMs / 1000).toFixed(2)}s`);

  const secondStarted = Date.now();
  const second = exec(tool, { spawnProcess });
  const secondTimer = setTimeout(() => second.child.kill('SIGKILL'), secondBoundMs);
  const secondOutcome = await second.done.finally(() => clearTimeout(secondTimer));
  const secondMs = Date.now() - secondStarted;
  if (secondOutcome.status !== 0 || secondOutcome.signal)
    throw new Error(
      `release-version second exec failed after ${secondMs}ms: ${secondOutcome.stderr}`
    );
  log(`release-version second exec took ${(secondMs / 1000).toFixed(2)}s`);
  return { firstMs, secondMs };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 2) throw new Error('Usage: warm-release-version.mjs');
    await warmReleaseVersion();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
