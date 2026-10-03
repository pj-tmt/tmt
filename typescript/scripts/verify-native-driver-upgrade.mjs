#!/usr/bin/env node
// Until #1084 adds released-package acquisition, prove the supported executable-path
// approval surface. The same current CLI approves both independently versioned archives.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { selectNativeArtifact, withNativeArtifact } from './native-artifact-policy.mjs';
import { assertNativeTarget, verifyNativeRuntime } from './native-runtime-proof.mjs';
import { runPackedCommand } from './packed-command.mjs';
import { compareVersions } from './release-versions.mjs';

export async function verifyDriverUpgrade(values) {
  const target = values.target;
  assertNativeTarget(target, 'Use matching-host artifacts');
  const current = selectNativeArtifact(values.manifest, values.archive, target, 'driver-herdr', {
    release: true,
  });
  const previous = selectNativeArtifact(
    values['previous-manifest'],
    values['previous-archive'],
    target,
    'driver-herdr'
  );
  const cli = selectNativeArtifact(
    values['driver-manifest'],
    values['driver-archive'],
    target,
    'cli'
  );
  assert(
    compareVersions(previous.version, current.version) < 0,
    'Use separately versioned artifacts, candidate newer than previous'
  );
  await withNativeArtifact(values['driver-archive'], cli, async (cliRoot) =>
    withNativeArtifact(values['previous-archive'], previous, async (oldRoot) =>
      withNativeArtifact(values.archive, current, async (nextRoot) => {
        const old = path.join(oldRoot, 'tmt-driver-herdr');
        const next = path.join(nextRoot, 'tmt-driver-herdr');
        for (const [executable, version] of [
          [old, previous.version],
          [next, current.version],
        ]) {
          await verifyNativeRuntime({
            executable,
            version,
            target,
            product: 'driver-herdr',
            subject: 'Driver upgrade archive',
          });
        }
        const oldBytes = fs.readFileSync(old);
        const nextBytes = fs.readFileSync(next);
        const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'tmt driver upgrade ')));
        try {
          const state = path.join(root, 'state');
          const env = { HOME: root, TMUX_TEAM_HOME: state, PATH: '', LANG: 'C', TMPDIR: root };
          const run = (args, expectedStatus = 0) =>
            JSON.parse(
              runPackedCommand(path.join(cliRoot, 'tmt'), [...args, '--json'], {
                cwd: root,
                env,
                expectedStatus,
              })
            );
          const list = () => run(['driver', 'ls']).drivers;
          const approve = (file, yes = true, status = 0) =>
            run(['driver', 'install', file, ...(yes ? ['--yes'] : [])], status);
          assert.equal(approve(old).approved.version, previous.version);
          assert.equal(list().find((driver) => driver.name === 'herdr')?.state, 'ok');
          const registry = path.join(state, 'drivers.json');
          const before = fs.readFileSync(registry);
          assert.equal(approve(next, false, 1).error.code, 'DRIVER_CONSENT_REQUIRED');
          assert(fs.readFileSync(registry).equals(before), 'Refused upgrade changed approval');
          const approved = approve(next).approved;
          assert.equal(approved.version, current.version);
          assert.equal(approved.path, fs.realpathSync(next));
          const listed = list().find((driver) => driver.name === 'herdr');
          assert.equal(listed?.version, current.version);
          assert.equal(listed?.sha256, approved.sha256);
          assert.equal(listed?.state, 'ok');
          assert.equal(
            approve(next).approved.sha256,
            approved.sha256,
            'Repeat approval changed payload'
          );
          assert(fs.readFileSync(old).equals(oldBytes), 'Previous driver bytes changed');
          assert(fs.readFileSync(next).equals(nextBytes), 'Candidate driver bytes changed');
          assert.equal(run(['driver', 'rm', 'herdr']).removed, 'herdr');
          assert.deepEqual(list(), [], 'Driver removal retained approval');
          assert(
            fs.readFileSync(old).equals(oldBytes) && fs.readFileSync(next).equals(nextBytes),
            'Approval removal deleted executable bytes'
          );
          assert(
            !fs.existsSync(path.join(state, 'tmux-team.db')),
            'Driver approval opened application SQLite'
          );
        } finally {
          fs.rmSync(root, { recursive: true, force: true });
        }
      })
    )
  );
  return { previous: previous.version, current: current.version };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({
    options: Object.fromEntries(
      [
        'archive',
        'manifest',
        'previous-archive',
        'previous-manifest',
        'driver-archive',
        'driver-manifest',
        'target',
        'product',
      ].map((key) => [key, { type: 'string' }])
    ),
  });
  for (const key of [
    'archive',
    'manifest',
    'previous-archive',
    'previous-manifest',
    'driver-archive',
    'driver-manifest',
    'target',
  ])
    assert(values[key], `--${key} is required`);
  assert(values.product === 'driver-herdr', 'Only the Herdr driver is released');
  const result = await verifyDriverUpgrade(values);
  console.log(
    `Driver approval upgrade verified: ${result.previous} -> ${result.current} (${values.target})`
  );
}
