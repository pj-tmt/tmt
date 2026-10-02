#!/usr/bin/env node
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { selectNativeArtifact, withNativeArtifact } from './native-artifact-policy.mjs';
import { assertNativeTarget, verifyNativeRuntime } from './native-runtime-proof.mjs';

const { values } = parseArgs({
  options: {
    manifest: { type: 'string' },
    archive: { type: 'string' },
    target: { type: 'string' },
    skill: { type: 'string' },
    notices: { type: 'string' },
    license: { type: 'string' },
    skills: { type: 'string' },
    product: { type: 'string', default: 'cli' },
  },
});
for (const name of [
  'manifest',
  'archive',
  'target',
  'notices',
  'license',
  ...(values.product === 'cli' ? ['skill'] : []),
  ...(values.product === 'squad' ? ['skills'] : []),
]) {
  assert(values[name], `--${name} is required`);
}
const metadata = selectNativeArtifact(
  values.manifest,
  values.archive,
  values.target,
  values.product,
  { release: true }
);
assertNativeTarget(values.target, 'Artifact requires a matching native host');
const skill = values.skill ? fs.readFileSync(values.skill, 'utf8') : undefined;
const inboxSkill =
  values.product === 'cli'
    ? fs.readFileSync(new URL('../../skills/tmt-inbox/SKILL.md', import.meta.url), 'utf8')
    : undefined;
const officeSkill =
  values.product === 'cli'
    ? fs.readFileSync(
        new URL('../../extensions/tmt-office/skills/tmt-office/SKILL.md', import.meta.url),
        'utf8'
      )
    : undefined;
const notices = fs.readFileSync(values.notices, 'utf8');
const executable = { cli: 'tmt', office: 'tmt-office', squad: 'tmt-squad' }[values.product];

/** Every regular file under `root`, by relative path; links fail. */
function tree(root) {
  const files = new Map();
  const walk = (directory, prefix) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
      const full = path.join(directory, entry.name);
      if (entry.isDirectory()) walk(full, relative);
      else {
        assert(entry.isFile(), `Skill tree entry must be a regular file: ${relative}`);
        files.set(relative, fs.readFileSync(full));
      }
    }
  };
  walk(root, '');
  return files;
}
assert(
  !/<year>|<copyright holders>/.test(notices),
  'Dependency notices contain placeholder attribution'
);

await withNativeArtifact(values.archive, metadata, async (artifactRoot) => {
  assert.equal(
    fs.readFileSync(path.join(artifactRoot, 'THIRD-PARTY-NOTICES.txt'), 'utf8'),
    notices,
    'Native archive notices differ from the generated inventory'
  );
  assert.deepEqual(
    fs.readFileSync(path.join(artifactRoot, 'LICENSE')),
    fs.readFileSync(values.license),
    'Native archive license differs from the selected source'
  );
  if (values.product === 'squad') {
    // The archived skills are exactly the reviewed sources, byte for byte.
    assert.deepEqual(
      tree(path.join(artifactRoot, 'skills')),
      tree(values.skills),
      'Native archive skills differ from their sources'
    );
  }
  await verifyNativeRuntime({
    executable: path.join(artifactRoot, executable),
    product: values.product,
    herdrDriver: values.product === 'cli' ? path.join(artifactRoot, 'tmt-driver-herdr') : undefined,
    target: metadata.target,
    version: metadata.version,
    skill,
    inboxSkill,
    officeSkill,
    squadSkill:
      values.product === 'squad'
        ? fs.readFileSync(path.join(values.skills, 'tmt-squad', 'SKILL.md'), 'utf8')
        : undefined,
    profileContent: 'Persisted by native archive',
    subject: 'Native archive',
    matchingHostMessage: 'Artifact requires a matching native host',
  });
  console.log(
    `Verified native archive ${metadata.name}: ${
      {
        cli: 'linkage, version, skill bundle, Herdr driver, managed install, SQLite persistence',
        office: 'linkage, exact Office handshake, no application state',
        squad: 'linkage, version, exact skills tree, no application state',
      }[values.product]
    }`
  );
});
