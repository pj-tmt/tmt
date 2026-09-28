#!/usr/bin/env node
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { verifyNativeRuntime } from './native-runtime-proof.mjs';

const { values } = parseArgs({
  options: {
    executable: { type: 'string' },
    target: { type: 'string' },
    version: { type: 'string' },
    skill: { type: 'string' },
  },
});
for (const name of ['executable', 'target', 'version', 'skill']) {
  assert(values[name], `--${name} is required`);
}

const executable = path.resolve(values.executable);
const skill = fs.readFileSync(values.skill, 'utf8');
const inboxSkill = fs.readFileSync(
  new URL('../../skills/tmt-inbox/SKILL.md', import.meta.url),
  'utf8'
);
const officeSkill = fs.readFileSync(
  new URL('../../extensions/tmt-office/skills/tmt-office/SKILL.md', import.meta.url),
  'utf8'
);
await verifyNativeRuntime({
  executable,
  target: values.target,
  version: values.version,
  skill,
  inboxSkill,
  officeSkill,
  profileContent: 'Persisted by native executable',
  subject: 'Native executable',
});
console.log(
  `Verified native executable ${executable}: linkage, version, skill bundle, managed install, SQLite persistence`
);
