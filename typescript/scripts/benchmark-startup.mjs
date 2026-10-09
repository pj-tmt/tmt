import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { runPackedCommand } from './packed-command.mjs';
import { resolveCliExecutables } from '../test/support/cli-executable.mjs';
import { assertBenchmarkHelp } from '../test/support/performance-contract.mjs';

// Resource measurement is deliberately macOS-only. Docker scenarios own tmux
// latency; do not compare these resource samples to Linux timing samples.
assert.equal(process.platform, 'darwin', 'This resource probe requires macOS /usr/bin/time.');
assert.equal(process.argv.length, 2, 'This probe does not accept arguments.');
const { cli } = resolveCliExecutables();
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-startup-baseline-'));
const samples = [];
try {
  const home = path.join(root, 'home');
  const bin = path.join(root, 'bin');
  fs.mkdirSync(home);
  fs.mkdirSync(bin);
  const forbidden = path.join(root, 'unexpected-tmux');
  fs.writeFileSync(path.join(bin, 'tmux'), '#!/bin/sh\ntouch "$TMT_BENCH_FORBIDDEN"\nexit 97\n', {
    mode: 0o755,
  });
  const env = {
    PATH: `${bin}:${process.env.PATH ?? ''}`,
    HOME: home,
    LC_ALL: 'C',
    TMT_HOME: path.join(home, 'global'),
    TMT_BENCH_FORBIDDEN: forbidden,
    TMT_BENCH_METRICS: path.join(root, 'metrics'),
    TMT_BENCH_STDERR: path.join(root, 'stderr'),
  };
  const measure = (scenario, args, verify) => {
    const started = performance.now();
    // Fixed shell program; all command arguments remain positional data. Keep
    // CLI diagnostics separate from time's stderr so neither masks the other.
    const stdout = runPackedCommand(
      '/bin/sh',
      [
        '-c',
        '/usr/bin/time -lp /bin/sh -c \'exec "$@" 2> "$TMT_BENCH_STDERR"\' bench "$@" 2> "$TMT_BENCH_METRICS"',
        'bench',
        cli.executable,
        ...cli.args,
        ...args,
      ],
      { cwd: root, env }
    );
    const wallMs = performance.now() - started;
    assert.equal(fs.readFileSync(env.TMT_BENCH_STDERR, 'utf8'), '');
    verify(stdout);
    const metrics = fs.readFileSync(env.TMT_BENCH_METRICS, 'utf8');
    const value = (pattern) => {
      const matches = [...metrics.matchAll(pattern)];
      assert.equal(matches.length, 1, `Missing or ambiguous resource metric: ${metrics}`);
      const number = Number(matches[0][1]);
      assert.ok(Number.isFinite(number) && number >= 0);
      return number;
    };
    samples.push({
      scenario,
      wallMs,
      userSeconds: value(/^user\s+(\d+(?:\.\d+)?)$/gm),
      systemSeconds: value(/^sys\s+(\d+(?:\.\d+)?)$/gm),
      maximumResidentSetBytes: value(/^\s*(\d+)\s+maximum resident set size$/gm),
    });
    assert.equal(fs.existsSync(forbidden), false, 'Storage/startup invoked tmux.');
  };
  for (let index = 0; index < 7; index += 1) {
    measure('help', ['--help'], assertBenchmarkHelp);
    env.TMT_HOME = path.join(home, `fresh-${index}`);
    let createdIdentity;
    measure('fresh-storage-create', ['--json', 'identity', 'create', 'Bench'], (stdout) => {
      const result = JSON.parse(stdout);
      assert.equal(result.created, true);
      assert.equal(result.identity.name, 'Bench');
      assert.equal(result.identity.canonicalName, 'bench');
      assert.match(result.identity.id, /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/);
      createdIdentity = result.identity;
    });
    measure('existing-storage-show', ['--json', 'identity', 'show', 'Bench'], (stdout) => {
      assert.deepEqual(JSON.parse(stdout).identity, createdIdentity);
    });
  }
  console.log(
    JSON.stringify({
      schemaVersion: 1,
      executable: cli,
      platform: process.platform,
      arch: process.arch,
      kernel: os.release(),
      node: process.version,
      cpu: os.cpus()[0]?.model,
      tool: '/usr/bin/time -lp',
      samples,
      interpretation:
        'Fresh processes, not dropped OS caches. CPU includes waited descendants; maximum RSS is not the sum of simultaneously live processes. Timing includes measurement wrappers. No tmux or agent work is measured.',
    })
  );
} finally {
  fs.rmSync(root, { recursive: true, force: true });
  assert.equal(fs.existsSync(root), false);
}
