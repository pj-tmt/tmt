#!/usr/bin/env node
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { generateNativeBootstrap } from './native-bootstrap.mjs';
import { selectNativeArtifact } from './native-artifact-policy.mjs';
import { runPackedCommand } from './packed-command.mjs';
import { assertMacOsArchitecture } from './native-runtime-proof.mjs';
import { bootstrapTools } from './native-bootstrap-proof.mjs';

const { values } = parseArgs({
  options: Object.fromEntries(
    ['manifest', 'archive', 'target', 'skill'].map((name) => [name, { type: 'string' }])
  ),
});
for (const name of ['manifest', 'archive', 'target', 'skill']) {
  assert(values[name], `--${name} is required`);
}
const metadata = selectNativeArtifact(values.manifest, values.archive, values.target, 'cli', {
  release: true,
});
const architecture = { arm64: 'aarch64', x64: 'x86_64' }[process.arch];
const platform = { darwin: 'apple-darwin', linux: 'unknown-linux-musl' }[process.platform];
assert(architecture && platform, 'Unsupported verification host');
assert.equal(values.target, `${architecture}-${platform}`, 'Use matching-host release artifacts');
const script = await generateNativeBootstrap(values.manifest, path.dirname(values.archive));
const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tmt bootstrap's ")));
try {
  const tools = bootstrapTools(root);
  const old = path.join(root, 'old npm bin');
  const prefix = path.join(root, 'native prefix');
  const state = path.join(root, 'app state');
  fs.mkdirSync(old);
  const oldBytes = '#!/bin/sh\nexit 99\n';
  fs.writeFileSync(path.join(old, 'tmt'), oldBytes, { mode: 0o755 });
  const installer = path.join(root, 'installer.sh');
  fs.writeFileSync(installer, script, { mode: 0o600 });
  const env = {
    HOME: root,
    TMPDIR: root,
    TMUX_TEAM_HOME: state,
    LANG: 'C',
    PATH: [old, tools].join(path.delimiter),
    TMT_FIXTURE_VERSION: metadata.version,
    TMT_FIXTURE_NAME: metadata.name,
    TMT_FIXTURE_MANIFEST: path.resolve(values.manifest),
    TMT_FIXTURE_ARCHIVE: path.resolve(values.archive),
  };
  const options = { cwd: root, env, timeoutMs: 30_000 };
  const bootstrap = (flags = []) =>
    runPackedCommand('/bin/sh', [installer, '--prefix', prefix, ...flags], options);
  const first = bootstrap(['--no-skill']);
  assert.match(first, /PATH may still select another installation/);
  assert.match(first, /npm uninstall -g tmux-team/);
  assert(!fs.existsSync(state), 'Binary-only bootstrap must not create application state');
  const executable = path.join(prefix, 'bin/tmt');
  assertMacOsArchitecture(executable, values.target, options);
  const run = (args) => runPackedCommand(executable, args, options);
  assert.equal(run(['--version']).trim(), metadata.version);
  const pointer = path.join(prefix, 'lib/tmux-team/current');
  const initial = fs.readlinkSync(pointer);
  bootstrap();
  assert.equal(fs.readlinkSync(pointer), initial, 'Repeat bootstrap must not create a release');
  assert.equal(run(['learn', '--skill']), fs.readFileSync(values.skill, 'utf8'));
  assert.equal(
    run(['learn', '--skill', 'tmt-office']),
    fs.readFileSync(
      new URL('../../extensions/tmt-office/skills/tmt-office/SKILL.md', import.meta.url),
      'utf8'
    )
  );
  const installedSkill = path.join(root, '.agents/skills/tmux-team/SKILL.md');
  const installedInboxSkill = path.join(root, '.agents/skills/tmt-inbox/SKILL.md');
  const installedOfficeSkill = path.join(root, '.agents/skills/tmt-office/SKILL.md');
  assert.equal(fs.readFileSync(installedSkill, 'utf8'), fs.readFileSync(values.skill, 'utf8'));
  assert.equal(
    fs.readFileSync(installedInboxSkill, 'utf8'),
    fs.readFileSync(new URL('../../skills/tmt-inbox/SKILL.md', import.meta.url), 'utf8')
  );
  assert(!fs.existsSync(installedOfficeSkill), 'CLI bootstrap must preserve core-only guidance');
  assert.deepEqual(
    JSON.parse(fs.readFileSync(path.join(state, 'skill-installations.json'), 'utf8')),
    {
      version: 1,
      targets: [
        path.join(root, '.agents/skills/tmt-inbox'),
        path.join(root, '.agents/skills/tmux-team'),
      ],
    }
  );
  assert(!fs.existsSync(path.join(state, 'tmux-team.db')), 'Skill setup must not open SQLite');
  bootstrap(['--pin']);
  const receipt = JSON.parse(
    fs.readFileSync(path.join(prefix, 'lib/tmux-team/current/receipt.json'))
  );
  assert.equal(receipt.pinned_version, metadata.version);
  const pinned = JSON.parse(run(['upgrade', '--json']));
  assert.equal(pinned.skippedPinned, true, 'Pinned managed executable must update without network');
  assert.equal(fs.readFileSync(path.join(old, 'tmt'), 'utf8'), oldBytes);
  // No tmt on PATH at all: a first install, with nothing to replace.
  const fresh = runPackedCommand('/bin/sh', [installer, '--no-skill'], {
    ...options,
    env: { ...env, PATH: tools },
  });
  assert.match(fresh, /\/\.local\/bin is not in PATH yet/);
  assert.doesNotMatch(fresh, /another installation|npm uninstall/);
  const defaultExecutable = path.join(root, '.local/bin/tmt');
  assertMacOsArchitecture(defaultExecutable, values.target, options);
  assert.equal(
    runPackedCommand(defaultExecutable, ['--version'], options).trim(),
    metadata.version
  );
  assert(
    !fs.readdirSync(root).some((name) => name.startsWith('tmt-bootstrap.')),
    'Bootstrap staging leaked'
  );
  console.log(
    `Verified actual native bootstrap: ${metadata.version} (${values.target}), isolated curl fixture, no Node/Rust runtime PATH`
  );
} finally {
  fs.rmSync(root, { recursive: true, force: true });
}
