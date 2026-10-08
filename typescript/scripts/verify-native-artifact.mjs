#!/usr/bin/env node
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { shipsSkills } from './component-skills.mjs';
import {
  assertDependencyNotices,
  readBoundedFile,
  selectNativeArtifact,
  withNativeArtifact,
} from './native-artifact-policy.mjs';
import { assertNativeTarget, verifyNativeRuntime } from './native-runtime-proof.mjs';
import { readSchemaEvidence, verifyApplicationSchema } from './native-application-schema.mjs';

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
    'source-root': { type: 'string' },
    'app-dir': { type: 'string' },
    'schema-snapshot': { type: 'string' },
    'schema-evidence': { type: 'string' },
  },
});
for (const name of [
  'manifest',
  'archive',
  'target',
  'notices',
  'license',
  ...(values.product === 'cli' ? ['skill'] : []),
  ...(shipsSkills(values.product) ? ['skills'] : []),
  ...(values.product === 'colab' ? ['app-dir'] : []),
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
const sourceRoot = values['source-root'] ?? fileURLToPath(new URL('../../', import.meta.url));
assert.equal(
  Boolean(values['schema-snapshot']),
  Boolean(values['schema-evidence']),
  'Both schema inputs required'
);
if (values['schema-snapshot'])
  assert.equal(values.product, 'cli', 'Schema verification is CLI-only');
const schemaManifest = values['schema-snapshot']
  ? readBoundedFile(values.manifest, 4 * 1024 * 1024)
  : undefined;
const inboxSkill =
  values.product === 'cli'
    ? fs.readFileSync(path.join(sourceRoot, 'skills/tmt-inbox/SKILL.md'), 'utf8')
    : undefined;
const officeSkill =
  values.product === 'cli'
    ? fs.readFileSync(
        path.join(sourceRoot, 'extensions/tmt-office/skills/tmt-office/SKILL.md'),
        'utf8'
      )
    : undefined;
const notices = fs.readFileSync(values.notices, 'utf8');
const executable = {
  cli: 'tmt',
  office: 'tmt-office',
  ops: 'tmt-ops',
  squad: 'tmt-squad',
  remote: 'tmt-remote',
  colab: 'tmt-colab',
  'driver-herdr': 'tmt-driver-herdr',
}[values.product];
let herdrVersion;
if (values.product === 'cli') {
  const source = values['source-root'] ?? fileURLToPath(new URL('../../', import.meta.url));
  const declaration = fs
    .readFileSync(path.join(source, 'rust/crates/tmt-driver-herdr/Cargo.toml'), 'utf8')
    .split(/\n\[/)[0];
  const explicit = /^version\s*=\s*"([^"]+)"$/m.exec(declaration)?.[1];
  assert(
    explicit || /^version\.workspace\s*=\s*true$/m.test(declaration),
    'Herdr version declaration is missing'
  );
  herdrVersion = explicit ?? metadata.version;
}

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
assertDependencyNotices(notices);

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
  if (shipsSkills(values.product)) {
    // The archived skills are exactly the reviewed sources, byte for byte.
    assert.deepEqual(
      tree(path.join(artifactRoot, 'skills')),
      tree(values.skills),
      'Native archive skills differ from their sources'
    );
  }
  if (schemaManifest) {
    const proof = verifyApplicationSchema({
      manifestBytes: schemaManifest,
      executable: path.join(artifactRoot, executable),
      archive: values.archive,
      target: values.target,
      root: sourceRoot,
      snapshot: JSON.parse(readBoundedFile(values['schema-snapshot'], 4 * 1024 * 1024)),
      evidence: readSchemaEvidence(values['schema-evidence']),
    });
    console.log(`Verified archived CLI schema: ${JSON.stringify(proof)}`);
  }
  await verifyNativeRuntime({
    executable: path.join(artifactRoot, executable),
    product: values.product,
    colabApp: values['app-dir'],
    notices,
    herdrDriver: values.product === 'cli' ? path.join(artifactRoot, 'tmt-driver-herdr') : undefined,
    herdrVersion,
    target: metadata.target,
    version: metadata.version,
    skill,
    inboxSkill,
    officeSkill,
    opsSkill:
      values.product === 'ops'
        ? fs.readFileSync(path.join(values.skills, 'tmt-ops', 'SKILL.md'), 'utf8')
        : undefined,
    squadSkill:
      values.product === 'squad'
        ? fs.readFileSync(path.join(values.skills, 'tmt-squad', 'SKILL.md'), 'utf8')
        : undefined,
    colabSkill:
      values.product === 'colab'
        ? fs.readFileSync(path.join(values.skills, 'tmt-colab', 'SKILL.md'), 'utf8')
        : undefined,
    profileContent: 'Persisted by native archive',
    subject: 'Native archive',
    matchingHostMessage: 'Artifact requires a matching native host',
  });
  if (schemaManifest)
    assert.deepEqual(
      readBoundedFile(values.manifest, 4 * 1024 * 1024),
      schemaManifest,
      'Final schema manifest changed during verification'
    );
  console.log(
    `Verified native archive ${metadata.name}: ${
      {
        cli: 'linkage, version, skill bundle, Herdr driver, managed install, SQLite persistence',
        office: 'linkage, exact Office handshake, no application state',
        ops: 'linkage, version, exact skills tree, no application state',
        squad: 'linkage, version, exact skills tree, no application state',
        'driver-herdr': 'linkage, exact capabilities/version, no application state',
        colab:
          'linkage, version, exact skills tree and embedded skill, relocated embedded app/assets and combined notices, socket cleanup',
      }[values.product]
    }`
  );
});
