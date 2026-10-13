import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, realpathSync, writeFileSync, symlinkSync, rmSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';
import assert from 'node:assert/strict';
import path from 'node:path';
import type { Sandbox } from './cli-process.js';

// Authored bundle from 691e909c938cf66d14c112a2ad92fc9a411dd4a4, immediately
// before the 2026-10-09 Core-name cutover. The compressed fixture records each
// Git blob and byte digest; it is never regenerated from the current catalog.
const legacyNames = [
  'tmux-team',
  'tmt-inbox',
  'tmt-office',
  'tmt-prop-create',
  'tmt-avatar-create',
];
export const officeNames = legacyNames.slice(2);
const providerRoots = [
  '.agents/skills',
  '.claude/skills',
  '.gemini/config/skills',
  '.pi/agent/skills',
];
type HistoricalBundle = {
  version: number;
  sourceCommit: string;
  framedSha256: string;
  skills: { name: string; base64: string; sha256: string }[];
};

function historicalBundle(): HistoricalBundle {
  const archive = readFileSync(
    new URL('../fixtures/managed-guidance-history.json.gz', import.meta.url)
  );
  assert.equal(
    createHash('sha256').update(archive).digest('hex'),
    '6e958ab9d9c842c8d19a3678b3d79533f51e5ade8e5a2441d0e9b0315f4f6d90'
  );
  const bundle = JSON.parse(gunzipSync(archive).toString('utf8')) as HistoricalBundle;
  assert.equal(bundle.version, 1);
  assert.equal(bundle.sourceCommit, '691e909c938cf66d14c112a2ad92fc9a411dd4a4');
  assert.deepEqual(
    bundle.skills.map((skill) => skill.name),
    legacyNames
  );
  return bundle;
}

export function oldLayout(sandbox: Sandbox, dangling = false, includeCore = false): string[] {
  const hash = createHash('sha256');
  const bundle = historicalBundle();
  const contents = bundle.skills.map((skill) => {
    const bytes = Buffer.from(skill.base64, 'base64');
    assert.equal(createHash('sha256').update(bytes).digest('hex'), skill.sha256);
    return bytes;
  });
  for (const bytes of contents) {
    const length = Buffer.alloc(8);
    length.writeBigUInt64BE(BigInt(bytes.length));
    hash.update(length).update(bytes);
  }
  mkdirSync(sandbox.globalDir, { recursive: true });
  assert.equal(hash.digest('hex'), bundle.framedSha256);
  const version = path.join(realpathSync(sandbox.globalDir), 'skill-assets', bundle.framedSha256);
  for (const [index, name] of legacyNames.entries()) {
    mkdirSync(path.join(version, name), { recursive: true });
    writeFileSync(path.join(version, name, 'SKILL.md'), contents[index]);
  }
  const targets = providerRoots.flatMap((root) =>
    officeNames.map((name) => path.join(sandbox.home, root, name))
  );
  const recordedTargets =
    includeCore && !dangling
      ? providerRoots.flatMap((root) =>
          legacyNames.map((name) => path.join(sandbox.home, root, name))
        )
      : targets;
  for (const target of recordedTargets) {
    mkdirSync(path.dirname(target), { recursive: true });
    symlinkSync(path.join(version, path.basename(target)), target);
  }
  writeFileSync(
    path.join(sandbox.globalDir, 'skill-installations.json'),
    JSON.stringify({ version: 1, targets: recordedTargets })
  );
  if (dangling) rmSync(version, { recursive: true });
  return targets;
}
