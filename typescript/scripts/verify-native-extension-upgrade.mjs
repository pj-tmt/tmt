#!/usr/bin/env node
// Proves an extension release (Office, Ops, Remote or Colab) upgrades from its previous published release with
// their real archives: same-product proofs use one newest published CLI; squad -> ops uses
// a pre-registration CLI for Squad and the newest supporting CLI for Ops. Install through
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
import { createHash } from 'node:crypto';
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
const PREVIOUS_DRIVER_OPTIONS = [
  'previous-product',
  'previous-driver-archive',
  'previous-driver-manifest',
];

/** Only the reviewed former-product contract may use a second CLI driver. */
export function extensionUpgradeOptions(args) {
  const { values } = parseArgs({
    args,
    options: Object.fromEntries(
      [...OPTIONS, ...PREVIOUS_DRIVER_OPTIONS].map((name) => [name, { type: 'string' }])
    ),
  });
  for (const name of OPTIONS) assert(values[name], `--${name} is required`);
  assert(
    ['office', 'ops', 'remote', 'colab'].includes(values.product),
    'Only extension products have this proof'
  );
  if (PREVIOUS_DRIVER_OPTIONS.some((name) => values[name] !== undefined)) {
    for (const name of PREVIOUS_DRIVER_OPTIONS)
      assert(values[name], `--${name} is required for replacement`);
    assert(
      values.product === 'ops' && values['previous-product'] === 'squad',
      'Only squad -> ops has a replacement proof'
    );
  }
  return values;
}

/** Pin the consumer contract's reported removal order, not internal lock/syscall timing. */
export function assertOpsReplacement(report, prefix, version) {
  assert.equal(report.extension, 'ops');
  assert.equal(report.installed, true);
  assert.equal(report.changed, true);
  assert.equal(report.version, version);
  assert.equal(report.executable, path.join(prefix, 'bin', 'tmt-ops'));
  assert.equal(report.replaced, 'squad');
  assert.deepEqual(
    report.removed,
    ['bin/tmt-squad', 'bin/tmt-sq', 'lib/tmt-squad'].map((entry) => path.join(prefix, entry))
  );
  assert.deepEqual(report.kept, []);
}

// Read only the verifier's private trees: hashes retain byte identity without large Buffer snapshots.
function treeInventory(root) {
  const entries = [];
  const visit = (directory, relative = '') => {
    for (const name of fs.readdirSync(directory).sort()) {
      const file = path.join(directory, name);
      const entry = path.join(relative, name);
      const stat = fs.lstatSync(file);
      if (stat.isSymbolicLink()) entries.push([entry, 'link', fs.readlinkSync(file)]);
      else if (stat.isDirectory()) {
        entries.push([entry, 'directory']);
        visit(file, entry);
      } else {
        assert(stat.isFile(), 'Proof state must contain only regular files, directories and links');
        entries.push([
          entry,
          'file',
          createHash('sha256').update(fs.readFileSync(file)).digest('hex'),
        ]);
      }
    }
  };
  visit(root);
  return entries;
}

export async function verifyExtensionUpgrade(values, runCommand = runPackedCommand) {
  const { product, target } = values;
  const crossProduct = values['previous-product'] !== undefined;
  const previousProduct = values['previous-product'] ?? product;
  // The archive under release is strict; the previous release and the CLI driver are published
  // archives, read against their own manifests.
  const current = selectNativeArtifact(values.manifest, values.archive, target, product, {
    release: true,
  });
  const previous = selectNativeArtifact(
    values['previous-manifest'],
    values['previous-archive'],
    target,
    previousProduct
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
  const previousDriverArtifact = crossProduct
    ? selectNativeArtifact(
        values['previous-driver-manifest'],
        values['previous-driver-archive'],
        target
      )
    : null;
  if (crossProduct) {
    assert(previous.skills && current.skills, 'Replacement proof requires both skill trees');
    assert(
      compareVersions(previousDriverArtifact.version, driverArtifact.version) < 0,
      'Previous CLI driver must be older'
    );
  }
  const architecture = { arm64: 'aarch64', x64: 'x86_64' }[process.arch];
  const platform = { darwin: 'apple-darwin', linux: 'unknown-linux-musl' }[process.platform];
  assert(architecture && platform, 'Unsupported verification host');
  assert.equal(target, `${architecture}-${platform}`, 'Use matching-host artifacts');
  const channel = current.version.includes('-') ? 'alpha' : 'stable';

  // The archive inventory, checksum and notices of both extension archives are checked before use.
  const candidateInventory = await withNativeArtifact(values.archive, current, async (source) => {
    assertMacOsArchitecture(path.join(source, `tmt-${product}`), target, {
      cwd: source,
      env: process.env,
    });
    return crossProduct
      ? {
          files: Object.fromEntries(
            treeInventory(source)
              .filter((entry) => entry[1] === 'file')
              .map(([file, , digest]) => [file, digest])
          ),
          skills: treeInventory(path.join(source, 'skills', 'tmt-ops')),
        }
      : null;
  });
  await withNativeArtifact(values['previous-archive'], previous, async (source) => {
    assertMacOsArchitecture(path.join(source, `tmt-${previousProduct}`), target, {
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
      const env = { HOME: root, TMUX_TEAM_HOME: state, PATH: '', LANG: 'C', TMPDIR: root };
      const driver = path.join(driverSource, 'tmt');
      assertMacOsArchitecture(driver, target, { cwd: root, env });
      const runWith = (executable, args, expectedStatus = 0) =>
        JSON.parse(runCommand(executable, args, { cwd: root, env, expectedStatus }));
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

      if (crossProduct) {
        const absent = (file) => {
          try {
            fs.lstatSync(file);
          } catch (error) {
            if (error.code === 'ENOENT') return;
            throw error;
          }
          assert.fail(`Entry must be absent: ${file}`);
        };
        const noCli = () => {
          for (const name of ['tmt', 'tmux-team']) absent(path.join(prefix, 'bin', name));
        };
        const owners = () =>
          JSON.parse(fs.readFileSync(path.join(state, 'skill-owners.json'), 'utf8'));
        const sentinels = new Map([
          ['ops.toml', '# retained user config\n'],
          ['ops/cron/jobs.json', '{"jobs":[],"literal":"tmt sq / tmt-squad"}\n'],
          ['ops/checklist/room/items.json', '{"items":[]}\n'],
        ]);
        const foreign = path.join(root, '.agents', 'skills', 'foreign', 'SKILL.md');
        const foreignBytes = 'Foreign provider content must remain byte-identical.\n';
        fs.mkdirSync(path.dirname(foreign), { recursive: true });
        fs.writeFileSync(foreign, foreignBytes);
        for (const [file, bytes] of sentinels) {
          const full = path.join(state, file);
          fs.mkdirSync(path.dirname(full), { recursive: true });
          fs.writeFileSync(full, bytes);
        }
        const preserved = () => {
          for (const [file, bytes] of sentinels)
            assert.equal(fs.readFileSync(path.join(state, file), 'utf8'), bytes);
          assert.equal(fs.readFileSync(foreign, 'utf8'), foreignBytes);
          noCli();
        };
        const argsFor = (name, archive, manifest, consent = false) => [
          'extension',
          'install',
          name,
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
          ...(consent ? ['--skills'] : []),
        ];
        let oldRecord;
        let oldGenerations;
        let oldTargets;
        await withNativeArtifact(
          values['previous-driver-archive'],
          previousDriverArtifact,
          async (oldSource) => {
            const oldDriver = path.join(oldSource, 'tmt');
            assertMacOsArchitecture(oldDriver, target, { cwd: root, env });
            assert.equal(
              runCommand(oldDriver, ['--version'], { cwd: root, env }).trim(),
              previousDriverArtifact.version
            );
            assert.equal(
              runCommand(driver, ['--version'], { cwd: root, env }).trim(),
              driverArtifact.version
            );
            const initial = runWith(
              oldDriver,
              argsFor('squad', values['previous-archive'], values['previous-manifest'], true)
            );
            assert.equal(initial.extension, 'squad');
            assert.equal(initial.installed, true);
            assert.equal(initial.changed, true);
            assert.equal(initial.version, previous.version);
            assert.equal(initial.executable, path.join(prefix, 'bin', 'tmt-squad'));
            const listed = runWith(oldDriver, ['extension', 'list', '--json', '--prefix', prefix]);
            assert.equal(
              listed.extensions.find(({ name }) => name === 'squad')?.version,
              previous.version
            );
            assert.equal(
              fs.readdirSync(path.join(prefix, 'lib', 'tmt-squad', 'releases')).length,
              1
            );
            for (const link of ['tmt-squad', 'tmt-sq']) {
              assert.equal(
                fs.realpathSync(path.join(prefix, 'bin', link)),
                fs.realpathSync(initial.executable)
              );
              assertMacOsArchitecture(path.join(prefix, 'bin', link), target, { cwd: root, env });
            }
            assert.deepEqual(initial.skills.available, ['tmt-squad']);
            oldRecord = owners().skills['tmt-squad'];
            assert.equal(oldRecord.owner, 'squad');
            oldTargets = oldRecord.targets;
            assert(oldTargets.length > 0, 'Initial install must record explicit skill consent');
            assert.deepEqual(
              initial.skills.published.map(({ target }) => target).sort(),
              [...oldTargets].sort()
            );
            for (const skillTarget of oldTargets) {
              assert(fs.lstatSync(skillTarget).isSymbolicLink(), 'Consented skill must be a link');
              assert(
                fs
                  .realpathSync(skillTarget)
                  .startsWith(path.join(state, 'skill-assets', 'owners', 'squad') + path.sep)
              );
            }
            oldGenerations = treeInventory(path.join(state, 'skill-assets', 'owners', 'squad'));
            preserved();
          }
        );

        const expectedTargets = oldTargets.map((entry) =>
          path.join(path.dirname(entry), 'tmt-ops')
        );
        const assertSkills = (report) => {
          assert.deepEqual(report.available, ['tmt-ops']);
          assert.deepEqual(report.removed, []);
          assert.deepEqual(
            report.published.map(({ target }) => target).sort(),
            [...expectedTargets].sort()
          );
          for (const entry of report.published) {
            assert.equal(entry.name, 'tmt-ops');
            assert.equal(entry.changed, false);
            assert.equal(entry.backup, null);
          }
          const record = owners();
          assert.equal(record.version, 1);
          assert.deepEqual(Object.keys(record.skills), ['tmt-ops']);
          assert.equal(record.skills['tmt-ops'].owner, 'ops');
          assert.deepEqual(
            [...record.skills['tmt-ops'].targets].sort(),
            [...expectedTargets].sort()
          );
          for (const skillTarget of oldTargets) absent(skillTarget);
          for (const skillTarget of expectedTargets) {
            assert(fs.lstatSync(skillTarget).isSymbolicLink(), 'Migrated skill must be a link');
            const immutable = path.join(
              state,
              'skill-assets',
              'owners',
              'ops',
              record.skills['tmt-ops'].digest,
              'tmt-ops'
            );
            assert.equal(fs.realpathSync(skillTarget), fs.realpathSync(immutable));
            assert.deepEqual(treeInventory(immutable), candidateInventory.skills);
          }
          assert.deepEqual(
            treeInventory(path.join(state, 'skill-assets', 'owners', 'squad')),
            oldGenerations
          );
        };
        const assertInstalled = () => {
          assert.equal(releases(), 1);
          for (const entry of ['bin/tmt-squad', 'bin/tmt-sq', 'lib/tmt-squad'])
            absent(path.join(prefix, entry));
          const active = fs.realpathSync(path.join(prefix, 'lib', 'tmt-ops', 'current'));
          assert.equal(
            fs.realpathSync(path.join(prefix, 'bin', 'tmt-ops')),
            path.join(active, 'tmt-ops')
          );
          const receipt = JSON.parse(fs.readFileSync(path.join(active, 'receipt.json'), 'utf8'));
          assert.equal(receipt.prefix, prefix);
          assert.equal(receipt.version, current.version);
          assert.equal(receipt.channel, channel);
          assert.equal(receipt.target, target);
          assert.equal(receipt.pinned_version, null);
          assert.equal(receipt.archive_sha256, current.sha256);
          assert.deepEqual(
            receipt.file_sha256,
            candidateInventory.files,
            'Receipt must inventory every candidate byte'
          );
          for (const [file, digest] of Object.entries(receipt.file_sha256)) {
            assert.equal(
              createHash('sha256')
                .update(fs.readFileSync(path.join(active, file)))
                .digest('hex'),
              digest
            );
          }
          assertMacOsArchitecture(path.join(prefix, 'bin', 'tmt-ops'), target, { cwd: root, env });
          const listed = run(['extension', 'list', '--json', '--prefix', prefix]);
          assert.deepEqual(listed.extensions, [
            {
              name: 'ops',
              installed: true,
              version: current.version,
              channel,
              pinned: null,
              commands: ['tmt-ops'],
              shadowedBy: [],
            },
            {
              name: 'remote',
              installed: false,
              version: null,
              channel: null,
              pinned: null,
              commands: ['tmt-remote'],
              shadowedBy: [],
            },
            {
              name: 'colab',
              installed: false,
              version: null,
              channel: null,
              pinned: null,
              commands: ['tmt-colab'],
              shadowedBy: [],
            },
          ]);
          preserved();
        };
        const upgraded = install(values.archive, values.manifest);
        assertOpsReplacement(upgraded, prefix, current.version);
        assertSkills(upgraded.skills);
        assertInstalled();
        const stable = [
          treeInventory(prefix),
          treeInventory(state),
          treeInventory(path.join(root, '.agents')),
        ];
        const unchanged = () =>
          assert.deepEqual(
            [
              treeInventory(prefix),
              treeInventory(state),
              treeInventory(path.join(root, '.agents')),
            ],
            stable
          );
        const repeat = install(values.archive, values.manifest);
        assert.equal(repeat.extension, 'ops');
        assert.equal(repeat.installed, true);
        assert.equal(repeat.changed, false);
        assert.equal(repeat.version, current.version);
        assert.equal(repeat.executable, upgraded.executable);
        for (const name of ['replaced', 'removed', 'kept'])
          assert(!Object.hasOwn(repeat, name), 'Repeat must omit replacement fields');
        assertSkills(repeat.skills);
        assertInstalled();
        unchanged();
        const refusal = install(values['previous-archive'], values['previous-manifest'], 1);
        assert.equal(refusal.error.code, 'EXTENSION_INSTALL_FAILED');
        assert.equal(
          refusal.error.message,
          'Native archive must belong to exactly one TMT release.'
        );
        assertInstalled();
        unchanged();
        console.log(
          `Extension replacement verified: squad ${previous.version} -> ops ${current.version} (${target})`
        );
        return;
      }

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
