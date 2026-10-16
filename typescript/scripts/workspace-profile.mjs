// Temporary #3231 observer. Never a test runner, selector, cleanup owner or rollout.
import { spawn, spawnSync } from 'node:child_process';
import {
  appendFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  readlinkSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';

export function parseStat(text, ticksPerSecond) {
  // comm may itself contain spaces and closing parentheses.
  const end = text.lastIndexOf(')');
  const fields = text
    .slice(end + 2)
    .trim()
    .split(/\s+/);
  if (end < 0 || fields.length < 22 || !(ticksPerSecond > 0))
    throw new Error('Incomplete proc stat');
  const number = (i) => {
    const value = Number(fields[i]);
    if (!Number.isFinite(value)) throw new Error('Invalid proc stat number');
    return value;
  };
  return {
    parent: number(1),
    startTicks: number(19),
    selfSeconds: (number(11) + number(12)) / ticksPerSecond,
    waitedSeconds: (number(13) + number(14)) / ticksPerSecond,
  };
}

export function parseCpu(text) {
  const lines = text.trim().split('\n');
  const fields = lines
    .find((line) => line.startsWith('cpu '))
    ?.trim()
    .split(/\s+/)
    .slice(1)
    .map(Number);
  if (!fields || fields.length < 8 || !fields.every(Number.isFinite))
    throw new Error('Incomplete proc CPU counters');
  // guest and guest_nice are already contained in user/nice.
  const total = fields.slice(0, 8).reduce((a, b) => a + b, 0);
  return {
    totalTicks: total,
    idleTicks: fields[3],
    ioWaitTicks: fields[4],
    running: Number(lines.find((line) => line.startsWith('procs_running '))?.split(' ')[1]),
  };
}

export function observeLine(state, line, seconds) {
  const clean = line.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '');
  const phase = clean.match(/^#3231 phase=([a-z-]+)$/);
  if (phase) {
    state.phases.push({ name: phase[1], seconds });
    state.phase = phase[1];
  }
  const native = clean.match(/^\s*Running (.+?) \(([^()]+)\)\s*$/);
  const doc = clean.match(/^\s*Doc-tests (\S+)\s*$/);
  if (native || doc) {
    const harness = {
      name: native?.[1] ?? doc[1],
      executable: native ? path.basename(native[2]) : null,
      kind: native ? 'native' : 'doctest',
      phase: state.phase,
      startSeconds: seconds,
      complete: false,
    };
    state.harnesses.push(harness);
    state.active = harness;
  }
  const result = clean.match(
    /^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;.*finished in ([\d.]+)s/
  );
  if (result && state.active) {
    Object.assign(state.active, {
      complete: true,
      endSeconds: seconds,
      wallSeconds: seconds - state.active.startSeconds,
      passed: Number(result[2]),
      failed: Number(result[3]),
      ignored: Number(result[4]),
      reportedSeconds: Number(result[5]),
    });
    state.active = null;
  }
}

function optionalRead(file) {
  try {
    return readFileSync(file, 'utf8');
  } catch {
    return null;
  }
}

export function readResourceSample(rootPid, target, ticks, phase, proc = '/proc') {
  if (!existsSync(`${proc}/stat`))
    return { unavailable: 'Linux procfs absent', phase, harnessProcesses: [] };
  const cpu = parseCpu(readFileSync(`${proc}/stat`, 'utf8'));
  const memory = Object.fromEntries(
    readFileSync(`${proc}/meminfo`, 'utf8')
      .split('\n')
      .filter((line) => /^(MemTotal|MemAvailable|SwapFree):/.test(line))
      .map((line) => {
        const [key, value] = line.split(/:\s*/);
        return [key, Number.parseInt(value, 10) * 1024];
      })
  );
  const vm = Object.fromEntries(
    readFileSync(`${proc}/vmstat`, 'utf8')
      .split('\n')
      .filter((line) => /^(pgpgin|pgpgout|pswpin|pswpout) /.test(line))
      .map((line) => {
        const [key, value] = line.split(' ');
        return [key, Number(value)];
      })
  );
  const processes = new Map();
  for (const entry of readdirSync(proc).filter((name) => /^[1-9]\d*$/.test(name))) {
    try {
      processes.set(Number(entry), parseStat(readFileSync(`${proc}/${entry}/stat`, 'utf8'), ticks));
    } catch {
      /* Process exited between directory/stat reads. */
    }
  }
  const descends = (pid, ancestor) => {
    const seen = new Set();
    while (processes.has(pid) && !seen.has(pid)) {
      if (pid === ancestor) return true;
      seen.add(pid);
      pid = processes.get(pid).parent;
    }
    return pid === ancestor;
  };
  const candidates = [];
  for (const [pid, stat] of processes) {
    if (!descends(pid, rootPid)) continue;
    let executable;
    try {
      executable = readlinkSync(`${proc}/${pid}/exe`);
    } catch {
      continue;
    }
    if (executable.startsWith(`${target}/debug/deps/`) && !executable.endsWith(' (deleted)'))
      candidates.push({ pid, executable, ...stat });
  }
  const roots = candidates.filter(
    (candidate) =>
      !candidates.some((other) => other.pid !== candidate.pid && descends(candidate.pid, other.pid))
  );
  const harnessProcesses = roots.map((root) => ({
    ...root,
    sampledInclusiveCpuSeconds: [...processes]
      .filter(([pid]) => descends(pid, root.pid))
      .reduce((sum, [, stat]) => sum + stat.selfSeconds + stat.waitedSeconds, 0),
  }));
  return {
    cpu,
    memory,
    vm,
    ioPressure: optionalRead(`${proc}/pressure/io`),
    harnessProcesses,
    phase,
  };
}

export function treeBytes(root) {
  let logicalBytes = 0,
    allocatedBytes = 0,
    files = 0;
  if (!existsSync(root)) return null;
  const visit = (file) => {
    const stat = lstatSync(file);
    if (stat.isDirectory()) for (const child of readdirSync(file)) visit(path.join(file, child));
    else {
      files += 1;
      logicalBytes += stat.size;
      allocatedBytes += stat.blocks * 512;
    }
  };
  visit(root);
  return { logicalBytes, allocatedBytes, files, followsSymlinks: false };
}

export async function measureCommand(
  output,
  argv,
  { intervalMs = 250, stdout = process.stdout } = {}
) {
  if (!argv.length || intervalMs < 20)
    throw new Error('Expected a command and bounded sample interval');
  mkdirSync(output, { recursive: false });
  const started = performance.now();
  const ownCpuStart = process.cpuUsage();
  const target = path.resolve('target');
  const ticksRead = spawnSync('getconf', ['CLK_TCK'], { encoding: 'utf8' });
  const ticks = Number(ticksRead.stdout.trim());
  if (!(ticks > 0)) throw new Error('Cannot determine CLK_TCK');
  const state = { phase: 'setup', phases: [], harnesses: [], active: null };
  const maxima = new Map();
  let sampleCount = 0,
    sampleError = null;
  const child = spawn(argv[0], argv.slice(1), { stdio: ['inherit', 'pipe', 'pipe'] });
  const sample = () => {
    try {
      const row = {
        seconds: (performance.now() - started) / 1000,
        ...readResourceSample(child.pid, target, ticks, state.phase),
      };
      appendFileSync(path.join(output, 'samples.jsonl'), `${JSON.stringify(row)}\n`);
      sampleCount += 1;
      for (const entry of row.harnessProcesses) {
        const key = `${state.phase}/${entry.pid}/${entry.startTicks}`;
        const prior = maxima.get(key);
        maxima.set(key, {
          ...entry,
          phase: state.phase,
          samples: (prior?.samples ?? 0) + 1,
          maxSampledCpuSeconds: Math.max(
            prior?.maxSampledCpuSeconds ?? 0,
            entry.sampledInclusiveCpuSeconds
          ),
        });
      }
    } catch (error) {
      sampleError = String(error);
    }
  };
  // A single merged child stream preserves Cargo Running/result order in CI.
  const pending = new Map();
  const record = (stream, chunk) => {
    stdout.write(chunk);
    appendFileSync(path.join(output, 'command.log'), chunk);
    const text = (pending.get(stream) ?? '') + chunk.toString('utf8');
    const lines = text.split('\n');
    pending.set(stream, lines.pop());
    for (const line of lines) observeLine(state, line, (performance.now() - started) / 1000);
  };
  child.stdout.on('data', (chunk) => record('stdout', chunk));
  child.stderr.on('data', (chunk) => record('stderr', chunk));
  sample();
  const timer = setInterval(sample, intervalMs);
  let spawnError;
  const result = await new Promise((resolve) => {
    child.once('error', (error) => {
      spawnError = String(error);
    });
    child.once('close', (code, signal) => resolve({ code, signal }));
  });
  clearInterval(timer);
  sample();
  for (const line of pending.values())
    if (line) observeLine(state, line, (performance.now() - started) / 1000);
  const wallSeconds = (performance.now() - started) / 1000;
  const ownCpu = process.cpuUsage(ownCpuStart);
  const processSamples = [...maxima.values()];
  for (const harness of state.harnesses) {
    const matches = processSamples.filter(
      (entry) =>
        entry.phase === harness.phase && path.basename(entry.executable) === harness.executable
    );
    harness.sampledCpuSeconds = matches.length
      ? matches.reduce((sum, entry) => sum + entry.maxSampledCpuSeconds, 0)
      : null;
    harness.processSamples = matches.reduce((sum, entry) => sum + entry.samples, 0);
  }
  const summary = {
    schemaVersion: 1,
    startedAt: new Date(Date.now() - wallSeconds * 1000).toISOString(),
    argv,
    cwd: process.cwd(),
    source: process.env.GITHUB_SHA ?? null,
    job: process.env.GITHUB_JOB ?? null,
    runner: process.env.RUNNER_NAME ?? null,
    imageVersion: process.env.ImageVersion ?? null,
    visibleCpuCount: os.cpus().length,
    availableParallelism: os.availableParallelism(),
    cpuQuota: optionalRead('/sys/fs/cgroup/cpu.max'),
    ticksPerSecond: ticks,
    intervalMs,
    wallSeconds,
    runnerMinutesOfCommand: wallSeconds / 60,
    samplerCpuSeconds: (ownCpu.user + ownCpu.system) / 1e6,
    sampleCount,
    sampleError,
    spawnError,
    ...result,
    phases: state.phases,
    harnesses: state.harnesses,
    processSamples,
    targetTree: treeBytes(target),
    interpretation:
      'CPU is sampled, includes live and waited descendants, may miss exit tails/short harnesses and races across proc reads; null is unavailable, never zero-filled. Host idle/iowait are global counters, not causal proof. Target tree is not an admitted transfer closure. Runner-minutes here cover the command only; REST job timestamps provide full-job and full-run occupancy. No process runner, isolation, selection, retry or cleanup policy is changed.',
  };
  writeFileSync(path.join(output, 'summary.json'), `${JSON.stringify(summary, null, 2)}\n`);
  if (spawnError) return 127;
  const status =
    result.code ?? (result.signal ? 128 + (os.constants.signals[result.signal] ?? 0) : 127);
  return status || (sampleError ? 65 : 0);
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const separator = process.argv.indexOf('--');
  if (separator !== 3)
    throw new Error('usage: workspace-profile.mjs <absent-output-dir> -- command [args]');
  process.exitCode = await measureCommand(path.resolve(process.argv[2]), process.argv.slice(4));
}
