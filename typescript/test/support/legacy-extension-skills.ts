import { createHash } from 'node:crypto';
import { mkdirSync, realpathSync, writeFileSync, symlinkSync, rmSync } from 'node:fs';
import path from 'node:path';
import type { Sandbox } from './cli-process.js';

// Layout published by the five-skill core bundle on 2026-09-30. These literal
// names and framed bytes are independent of the current product catalog.
const legacyNames = ['tmt', 'tmt-inbox', 'tmt-office', 'tmt-prop-create', 'tmt-avatar-create'];
export const officeNames = legacyNames.slice(2);
const providerRoots = [
  '.agents/skills',
  '.claude/skills',
  '.gemini/config/skills',
  '.pi/agent/skills',
];
export function oldLayout(sandbox: Sandbox, dangling = false): string[] {
  const hash = createHash('sha256');
  const contents = legacyNames.map((name) =>
    Buffer.from(`---\nname: ${name}\n---\nLegacy fixture.\n`)
  );
  for (const bytes of contents) {
    const length = Buffer.alloc(8);
    length.writeBigUInt64BE(BigInt(bytes.length));
    hash.update(length).update(bytes);
  }
  mkdirSync(sandbox.globalDir, { recursive: true });
  const version = path.join(realpathSync(sandbox.globalDir), 'skill-assets', hash.digest('hex'));
  for (const [index, name] of legacyNames.entries()) {
    mkdirSync(path.join(version, name), { recursive: true });
    writeFileSync(path.join(version, name, 'SKILL.md'), contents[index]);
  }
  const targets = providerRoots.flatMap((root) =>
    officeNames.map((name) => path.join(sandbox.home, root, name))
  );
  for (const target of targets) {
    mkdirSync(path.dirname(target), { recursive: true });
    symlinkSync(path.join(version, path.basename(target)), target);
  }
  writeFileSync(
    path.join(sandbox.globalDir, 'skill-installations.json'),
    JSON.stringify({ version: 1, targets })
  );
  if (dangling) rmSync(version, { recursive: true });
  return targets;
}
