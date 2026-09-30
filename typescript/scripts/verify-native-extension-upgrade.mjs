#!/usr/bin/env node
// Proves an extension release (Office or Squad) upgrades from its previous published release with
// their real archives: the newest published CLI installs the previous archive with
// `tmt <extension> install`, then the candidate over it, in an isolated home, state and prefix.
// The extension archives carry no installer of their own, so the CLI is the driver.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { selectNativeArtifact, withNativeArtifact } from './native-artifact-policy.mjs';
import { runPackedCommand } from './packed-command.mjs';
import { compareVersions } from './release-versions.mjs';

const OPTIONS = [
  'product',
  'archive',
  'manifest',
  'previous-archive',
  'previous-manifest',
  'driver-archive',
  'driver-manifest',
  'target',
];
const { values } = parseArgs({
  options: Object.fromEntries(OPTIONS.map((name) => [name, { type: 'string' }])),
});
for (const name of OPTIONS) assert(values[name], `--${name} is required`);
assert(['office', 'squad'].includes(values.product), 'Only extension products have this proof');

const { product, target } = values;
const current = selectNativeArtifact(values.manifest, values.archive, target, product);
const previous = selectNativeArtifact(
  values['previous-manifest'],
  values['previous-archive'],
  target,
  product
);
const driverArtifact = selectNativeArtifact(
  values['driver-manifest'],
  values['driver-archive'],
  target
);
assert(
  compareVersions(previous.version, current.version) < 0,
  'Use actual separately versioned release artifacts, the candidate newer than the previous'
);
const architecture = { arm64: 'aarch64', x64: 'x86_64' }[process.arch];
const platform = { darwin: 'apple-darwin', linux: 'unknown-linux-musl' }[process.platform];
assert(architecture && platform, 'Unsupported verification host');
assert.equal(target, `${architecture}-${platform}`, 'Use matching-host artifacts');
const channel = current.version.includes('-') ? 'alpha' : 'stable';

// The archive inventory, checksum and notices of both extension archives are checked before use.
await withNativeArtifact(values.archive, current, async () => {});
await withNativeArtifact(values['previous-archive'], previous, async () => {});
await withNativeArtifact(values['driver-archive'], driverArtifact, async (driverSource) => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tmt extension's upgrade ")));
  try {
    const prefix = path.join(root, 'prefix with spaces');
    const state = path.join(root, 'separate application state');
    const env = { HOME: root, TMUX_TEAM_HOME: state, PATH: '', LANG: 'C', TMPDIR: root };
    const driver = path.join(driverSource, 'tmt');
    const run = (args, expectedStatus = 0) =>
      JSON.parse(runPackedCommand(driver, args, { cwd: root, env, expectedStatus }));
    const install = (archive, manifest, expectedStatus = 0) =>
      run(
        [
          product,
          'install',
          '--yes',
          '--json',
          '--archive',
          path.resolve(archive),
          '--manifest',
          path.resolve(manifest),
          '--prefix',
          prefix,
          '--channel',
          channel,
        ],
        expectedStatus
      );
    const status = () => run([product, 'status', '--json', '--prefix', prefix]);
    const releases = () =>
      fs.readdirSync(path.join(prefix, 'lib', `tmt-${product}`, 'releases')).length;

    const initial = install(values['previous-archive'], values['previous-manifest']);
    assert.equal(initial.installed, true);
    assert.equal(initial.changed, true);
    assert.equal(initial.version, previous.version);
    assert.equal(status().version, previous.version);
    assert.equal(releases(), 1);

    const upgraded = install(values.archive, values.manifest);
    assert.equal(upgraded.changed, true);
    assert.equal(upgraded.version, current.version);
    assert.equal(status().version, current.version);
    assert.equal(releases(), 2, 'The previous release must stay on disk after the upgrade');
    assert(
      !fs.existsSync(path.join(prefix, 'bin', 'tmt')),
      'An extension install must not create the CLI link'
    );

    assert.equal(install(values.archive, values.manifest).changed, false);
    const downgrade = install(values['previous-archive'], values['previous-manifest'], 1);
    assert.match(downgrade.error.message, /downgrade/);
    assert.equal(status().version, current.version);
    assert.equal(releases(), 2);
    console.log(
      `Extension upgrade verified: ${product} ${previous.version} -> ${current.version} (${target})`
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
