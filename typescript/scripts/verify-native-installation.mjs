#!/usr/bin/env node
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { selectNativeArtifact, withNativeArtifact } from './native-artifact-policy.mjs';
import { runPackedCommand } from './packed-command.mjs';

const { values } = parseArgs({
  options: Object.fromEntries(
    ['archive', 'manifest', 'previous-archive', 'previous-manifest', 'target', 'skill'].map(
      (name) => [name, { type: 'string' }]
    )
  ),
});
for (const name of [
  'archive',
  'manifest',
  'previous-archive',
  'previous-manifest',
  'target',
  'skill',
]) {
  assert(values[name], `--${name} is required`);
}
const current = selectNativeArtifact(values.manifest, values.archive, values.target);
const previous = selectNativeArtifact(
  values['previous-manifest'],
  values['previous-archive'],
  values.target
);
assert.notEqual(
  previous.version,
  current.version,
  'Use actual separately versioned release artifacts'
);
const architecture = { arm64: 'aarch64', x64: 'x86_64' }[process.arch];
const platform = { darwin: 'apple-darwin', linux: 'unknown-linux-musl' }[process.platform];
assert(architecture && platform, 'Unsupported verification host');
assert.equal(values.target, `${architecture}-${platform}`, 'Use matching-host artifacts');

await withNativeArtifact(values.archive, current, async (source) => {
  await withNativeArtifact(values['previous-archive'], previous, async (oldSource) => {
    const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tmt managed install's ")));
    try {
      const prefix = path.join(root, 'prefix with spaces');
      const state = path.join(root, 'separate application state');
      const env = { HOME: root, TMUX_TEAM_HOME: state, PATH: '', LANG: 'C', TMPDIR: root };
      const options = { cwd: root, env };
      const installer = path.join(source, 'tmt');
      assert.equal(runPackedCommand(installer, ['--version'], options).trim(), current.version);
      assert.equal(
        runPackedCommand(path.join(oldSource, 'tmt'), ['--version'], options).trim(),
        previous.version
      );
      const managed = path.join(prefix, 'bin/tmt');
      const pointer = path.join(prefix, 'lib/tmux-team/current');
      const run = (args) => runPackedCommand(managed, args, options);
      const install = (archive, manifest, flags = [], expectedStatus = 0) =>
        JSON.parse(
          runPackedCommand(
            installer,
            [
              '__native-install',
              '--archive',
              path.resolve(archive),
              '--manifest',
              path.resolve(manifest),
              '--prefix',
              prefix,
              '--channel',
              'alpha',
              ...flags,
              '--json',
            ],
            { ...options, expectedStatus }
          )
        );
      const initial = install(values['previous-archive'], values['previous-manifest'], ['--pin']);
      assert.equal(initial.changed, true);
      assert.equal(run(['--version']).trim(), previous.version);
      assert(
        !fs.existsSync(state),
        'Binary installation must not discover or create application state'
      );
      const originalPointer = fs.readlinkSync(pointer);
      const identity = JSON.parse(
        run(['identity', 'create', 'installation-proof', '--json'])
      ).identity;
      const database = path.join(state, 'tmux-team.db');
      const originalDatabase = fs.readFileSync(database);

      const blocked = install(values.archive, values.manifest, [], 1);
      assert.equal(blocked.error.code, 'NATIVE_INSTALL_FAILED');
      assert.match(blocked.error.message, /pinned/);
      assert.equal(fs.readlinkSync(pointer), originalPointer);
      assert.deepEqual(fs.readFileSync(database), originalDatabase);

      const upgraded = install(values.archive, values.manifest, ['--unpin']);
      assert.equal(upgraded.changed, true);
      assert.notEqual(fs.readlinkSync(pointer), originalPointer);
      assert.equal(run(['--version']).trim(), current.version);
      assert.equal(run(['learn', '--skill']), fs.readFileSync(values.skill, 'utf8'));
      assert.equal(
        run(['learn', '--skill', 'tmt-office']),
        fs.readFileSync(
          new URL('../../extensions/tmt-office/skills/tmt-office/SKILL.md', import.meta.url),
          'utf8'
        )
      );
      assert.equal(
        run(['learn', '--skill', 'tmt-avatar-create']),
        fs.readFileSync(
          new URL('../../extensions/tmt-office/skills/tmt-avatar-create/SKILL.md', import.meta.url),
          'utf8'
        )
      );
      assert.deepEqual(
        fs.readFileSync(database),
        originalDatabase,
        'Installation must not migrate or write SQLite'
      );
      assert.deepEqual(
        JSON.parse(run(['identity', 'show', 'installation-proof', '--json'])).identity,
        identity
      );
      const oldExecutable = path.join(prefix, 'lib/tmux-team', originalPointer, 'tmt');
      assert.equal(
        runPackedCommand(oldExecutable, ['--version'], options).trim(),
        previous.version
      );

      const activePointer = fs.readlinkSync(pointer);
      assert.equal(install(values.archive, values.manifest).changed, false);
      const downgrade = install(
        values['previous-archive'],
        values['previous-manifest'],
        ['--pin'],
        1
      );
      assert.match(downgrade.error.message, /downgrade/);
      assert.equal(fs.readlinkSync(pointer), activePointer);
      assert.equal(run(['--version']).trim(), current.version);
      assert.equal(fs.readdirSync(path.join(prefix, 'lib/tmux-team/releases')).length, 2);
      console.log(
        `Managed native lifecycle verified: ${previous.version} -> ${current.version} (${values.target})`
      );
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
});
