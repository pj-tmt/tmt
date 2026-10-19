#!/usr/bin/env node
// Proves an extension release (Ops, Remote, Colab or Digest) upgrades from its previous published release with
// their real archives, driven by the newest published CLI. Install through
// `tmt extension install <extension>`, then the candidate over it, in an isolated home, state and
// prefix. The extension archives carry no installer of their own, so the CLI is the driver, and
// `tmt extension install|list` is the one surface that drives every extension: no extension has an
// install command of its own under `tmt <extension>`.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';
import { selectNativeArtifact, withNativeArtifact } from './native-artifact-policy.mjs';
import { runPackedCommand } from './packed-command.mjs';
import { compareVersions } from './release-versions.mjs';
import { assertMacOsArchitecture } from './native-runtime-proof.mjs';

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
/** Validate the same-product archive proof inputs. */
export function extensionUpgradeOptions(args) {
  const { values } = parseArgs({
    args,
    options: Object.fromEntries(OPTIONS.map((name) => [name, { type: 'string' }])),
  });
  for (const name of OPTIONS) assert(values[name], `--${name} is required`);
  assert(
    ['ops', 'remote', 'colab', 'digest'].includes(values.product),
    'Only extension products have this proof'
  );
  return values;
}

export async function verifyExtensionUpgrade(values, runCommand = runPackedCommand) {
  const { product, target } = values;
  // The archive under release is strict; the previous release and the CLI driver are published
  // archives, read against their own manifests.
  const current = selectNativeArtifact(values.manifest, values.archive, target, product, {
    release: true,
  });
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
  await withNativeArtifact(values.archive, current, async (source) => {
    assertMacOsArchitecture(path.join(source, `tmt-${product}`), target, {
      cwd: source,
      env: process.env,
    });
  });
  await withNativeArtifact(values['previous-archive'], previous, async (source) => {
    assertMacOsArchitecture(path.join(source, `tmt-${product}`), target, {
      cwd: source,
      env: process.env,
    });
  });
  await withNativeArtifact(values['driver-archive'], driverArtifact, async (driverSource) => {
    const root = fs.realpathSync(
      fs.mkdtempSync(path.join(os.tmpdir(), "tmt extension's upgrade "))
    );
    try {
      const prefix = path.join(root, 'prefix with spaces');
      const state = path.join(root, 'separate application state');
      const env = {
        HOME: root,
        TMT_HOME: state,
        PATH: '',
        LANG: 'C',
        TMPDIR: root,
      };
      const driver = path.join(driverSource, 'tmt');
      assertMacOsArchitecture(driver, target, { cwd: root, env });
      const run = (args, expectedStatus = 0) =>
        JSON.parse(runCommand(driver, args, { cwd: root, env, expectedStatus }));
      const install = (archive, manifest, expectedStatus = 0) =>
        run(
          [
            'extension',
            'install',
            product,
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
      const installedVersion = () =>
        run(['extension', 'list', '--json', '--prefix', prefix]).extensions.find(
          ({ name }) => name === product
        )?.version;
      const releases = () =>
        fs.readdirSync(path.join(prefix, 'lib', `tmt-${product}`, 'releases')).length;

      const initial = install(values['previous-archive'], values['previous-manifest']);
      assertMacOsArchitecture(path.join(prefix, 'bin', `tmt-${product}`), target, {
        cwd: root,
        env,
      });
      assert.equal(initial.extension, product);
      assert.equal(initial.installed, true);
      assert.equal(initial.changed, true);
      assert.equal(initial.version, previous.version);
      assert.equal(installedVersion(), previous.version);
      assert.equal(releases(), 1);

      const upgraded = install(values.archive, values.manifest);
      assertMacOsArchitecture(path.join(prefix, 'bin', `tmt-${product}`), target, {
        cwd: root,
        env,
      });
      assert.equal(upgraded.changed, true);
      assert.equal(upgraded.version, current.version);
      assert.equal(installedVersion(), current.version);
      assert.equal(releases(), 2, 'The previous release must stay on disk after the upgrade');
      assert(
        !fs.existsSync(path.join(prefix, 'bin', 'tmt')),
        'An extension install must not create the CLI link'
      );

      assert.equal(install(values.archive, values.manifest).changed, false);
      const downgrade = install(values['previous-archive'], values['previous-manifest'], 1);
      assert.match(downgrade.error.message, /downgrade/);
      assert.equal(installedVersion(), current.version);
      assert.equal(releases(), 2);
      console.log(
        `Extension upgrade verified: ${product} ${previous.version} -> ${current.version} (${target})`
      );
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await verifyExtensionUpgrade(extensionUpgradeOptions(process.argv.slice(2)));
}
