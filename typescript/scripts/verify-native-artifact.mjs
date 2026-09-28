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
]) {
  assert(values[name], `--${name} is required`);
}
const metadata = selectNativeArtifact(
  values.manifest,
  values.archive,
  values.target,
  values.product
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
  await verifyNativeRuntime({
    executable: path.join(artifactRoot, values.product === 'cli' ? 'tmt' : 'tmt-office'),
    product: values.product,
    target: metadata.target,
    version: metadata.version,
    skill,
    inboxSkill,
    officeSkill,
    profileContent: 'Persisted by native archive',
    subject: 'Native archive',
    matchingHostMessage: 'Artifact requires a matching native host',
  });
  console.log(
    `Verified native archive ${metadata.name}: ${values.product === 'cli' ? 'linkage, version, skill bundle, managed install, SQLite persistence' : 'linkage, exact Office handshake, no application state'}`
  );
});
