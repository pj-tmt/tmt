import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it } from 'vite-plus/test';
import {
  measureCommand,
  observeLine,
  parseCpu,
  parseStat,
  readResourceSample,
  treeBytes,
  type ProfileState,
} from '../../scripts/workspace-profile.mjs';
const owned: string[] = [];
const fixture = () => {
  const root = mkdtempSync(path.join(tmpdir(), 'tmt-workspace-profile-'));
  owned.push(root);
  return root;
};
afterEach(() => {
  for (const root of owned.splice(0)) rmSync(root, { recursive: true, force: true });
});
it('reads process accounting after the final comm parenthesis', () => {
  const fields = Array.from({ length: 22 }, () => '0');
  fields[0] = 'S';
  fields[1] = '42';
  fields[11] = '120';
  fields[12] = '30';
  fields[13] = '100';
  fields[14] = '50';
  fields[19] = '999';
  expect(parseStat(`123 (name ) with spaces) ${fields.join(' ')}`, 100)).toEqual({
    parent: 42,
    startTicks: 999,
    selfSeconds: 1.5,
    waitedSeconds: 1.5,
  });
  expect(() => parseStat('truncated', 100)).toThrow('Incomplete');
  expect(() => parseStat(`123 (ok) ${fields.join(' ')}`, 0)).toThrow('Incomplete');
});
it('does not double-count guest CPU or call iowait idle CPU', () => {
  expect(parseCpu('cpu 10 20 30 40 50 60 70 80 90 100\nprocs_running 3\n')).toEqual({
    totalTicks: 360,
    idleTicks: 40,
    ioWaitTicks: 50,
    running: 3,
  });
  expect(() => parseCpu('cpu 1 2\n')).toThrow('Incomplete');
});
it('separates discovery, native harnesses and doctests and retains failures', () => {
  const state: ProfileState = { phase: 'setup', phases: [], harnesses: [], active: null };
  observeLine(state, '#3231 phase=remote-list', 0);
  observeLine(state, ' Running unittests src/lib.rs (target/debug/deps/tmt_remote-aaa)', 1);
  observeLine(state, '#3231 phase=harnesses', 2);
  observeLine(state, ' Running unittests src/lib.rs (target/debug/deps/tmt_remote-aaa)', 3);
  observeLine(
    state,
    'test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.25s',
    4.5
  );
  observeLine(state, '   Doc-tests tmt_core', 5);
  observeLine(
    state,
    'test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s',
    5.2
  );
  expect(
    state.harnesses.map(({ kind, phase, complete, failed }) => ({ kind, phase, complete, failed }))
  ).toEqual([
    { kind: 'native', phase: 'remote-list', complete: false, failed: undefined },
    { kind: 'native', phase: 'harnesses', complete: true, failed: 0 },
    { kind: 'doctest', phase: 'harnesses', complete: true, failed: 1 },
  ]);
  expect(state.harnesses[1]).toMatchObject({
    wallSeconds: 1.5,
    reportedSeconds: 1.25,
    executable: 'tmt_remote-aaa',
    passed: 3,
    ignored: 1,
  });
});
it('sizes a tree without following foreign symlinks', () => {
  const root = fixture();
  writeFileSync(path.join(root, 'artifact'), '12345');
  symlinkSync('/unowned-missing-target', path.join(root, 'link'));
  expect(treeBytes(root)).toMatchObject({
    logicalBytes: 5 + '/unowned-missing-target'.length,
    files: 2,
    followsSymlinks: false,
  });
  expect(treeBytes(path.join(root, 'absent'))).toBeNull();
});
it.each([0, 7])('preserves a real command exit %i and its exact output', async (status) => {
  const output = path.join(fixture(), 'evidence');
  const chunks: Uint8Array[] = [];
  const text =
    '#3231 phase=harnesses\n Running unittests src/lib.rs (target/debug/deps/probe-abc)\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n';
  expect(
    await measureCommand(
      output,
      [
        process.execPath,
        '-e',
        `process.stdout.write(${JSON.stringify(text)}); process.exitCode = ${status};`,
      ],
      { stdout: { write: (chunk) => chunks.push(chunk) } }
    )
  ).toBe(status);
  expect(Buffer.concat(chunks).toString()).toBe(text);
  expect(readFileSync(path.join(output, 'command.log'), 'utf8')).toBe(text);
  const report = JSON.parse(readFileSync(path.join(output, 'summary.json'), 'utf8'));
  expect(report.code).toBe(status);
  expect(report.harnesses[0]).toMatchObject({ complete: true, passed: 1, sampledCpuSeconds: null });
  expect(report.availableParallelism).toBeGreaterThan(0);
  expect(report.wallSeconds).toBeGreaterThan(0);
  expect(report.sampleCount).toBeGreaterThanOrEqual(2);
  expect(report.samplerCpuSeconds).toBeGreaterThanOrEqual(0);
});
it('retains a spawn failure and its evidence', async () => {
  const output = path.join(fixture(), 'evidence');
  expect(
    await measureCommand(output, ['/no/such/workspace-profile-command'], {
      stdout: { write: () => {} },
    })
  ).toBe(127);
  expect(JSON.parse(readFileSync(path.join(output, 'summary.json'), 'utf8')).spawnError).toContain(
    'ENOENT'
  );
});
it('keeps nextest read-only, PR-only and outside required aggregates', () => {
  const workflow = readFileSync(
    new URL('../../../.github/workflows/ci.yml', import.meta.url),
    'utf8'
  );
  const job = workflow
    .split('\n  workspace-nextest-trial:\n')[1]
    .split('\n  native-process-tests:')[0];
  for (const text of [
    "if: github.event_name == 'pull_request' && needs.changes.outputs.native_scope == 'full'",
    'contents: read',
    'save-if: false',
    'cargo nextest run --locked --workspace --no-fail-fast',
    'cargo test --locked --workspace --doc',
    'cargo build --locked --workspace --profile test --tests --timings',
    'cargo nextest archive --locked --workspace',
    'cargo nextest list --archive-file',
    '223ba936714cc861a9cdedad5a8eb664dfdd69d6fbdf6cea6aa8184f523290dc',
    'retention-days: 1',
  ])
    expect(job).toContain(text);
  expect(job).not.toContain('continue-on-error');
  for (const aggregate of ['native-rust', 'native-package', 'complete-main']) {
    const body = workflow.split(`\n  ${aggregate}:\n`)[1]?.split(/\n {2}[a-z0-9-]+:\n/)[0];
    if (body) expect(body).not.toContain('workspace-nextest-trial');
  }
});
it('preserves every baseline Cargo command without a runner override', () => {
  const workflow = readFileSync(
    new URL('../../../.github/workflows/ci.yml', import.meta.url),
    'utf8'
  );
  const body = workflow
    .split("<<'WORKSPACE'\n")[1]
    .split('          WORKSPACE\n')[0]
    .replace(/^ {10}/gm, '')
    .split('\n')
    .filter((line) => line && line !== 'exec 2>&1' && !line.startsWith("printf '#3231 phase="));
  expect(body).toEqual([
    'remote_tests="$(cargo test --locked -p tmt-remote -- --list)"',
    'printf \'%s\\n\' "$remote_tests"',
    'grep -q \': test$\' <<< "$remote_tests"',
    'node ../typescript/scripts/private-world.mjs -- cargo test --locked --workspace',
    'cargo build --locked --workspace',
  ]);
});

it('samples only descendant harness roots and includes live and waited child CPU once', () => {
  const proc = fixture();
  writeFileSync(path.join(proc, 'stat'), 'cpu 10 20 30 40 50 60 70 80 90 100\nprocs_running 3\n');
  writeFileSync(
    path.join(proc, 'meminfo'),
    'MemTotal: 1000 kB\nMemAvailable: 800 kB\nSwapFree: 0 kB\n'
  );
  writeFileSync(path.join(proc, 'vmstat'), 'pgpgin 7\npgpgout 8\n');
  const processFixture = (
    pid: number,
    parent: number,
    selfTicks: number,
    waitedTicks: number,
    exe?: string
  ) => {
    mkdirSync(path.join(proc, String(pid)));
    const fields = Array.from({ length: 22 }, () => '0');
    fields[0] = 'S';
    fields[1] = String(parent);
    fields[11] = String(selfTicks);
    fields[13] = String(waitedTicks);
    fields[19] = String(pid * 10);
    writeFileSync(path.join(proc, String(pid), 'stat'), `${pid} (fixture) ${fields.join(' ')}`);
    if (exe) symlinkSync(exe, path.join(proc, String(pid), 'exe'));
  };
  processFixture(10, 1, 0, 0);
  processFixture(11, 10, 200, 100, '/checkout/target/debug/deps/harness-abc');
  processFixture(12, 11, 400, 50, '/checkout/target/debug/tmt');
  processFixture(99, 1, 9000, 0, '/checkout/target/debug/deps/foreign-abc');
  expect(
    readResourceSample(10, '/checkout/target', 100, 'harnesses', proc).harnessProcesses
  ).toEqual([
    expect.objectContaining({ pid: 11, startTicks: 110, sampledInclusiveCpuSeconds: 7.5 }),
  ]);
});
