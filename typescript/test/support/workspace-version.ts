import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const manifest = fileURLToPath(new URL('../../../rust/Cargo.toml', import.meta.url));

/**
 * The version `tmt --version` prints: `version` in the `[workspace.package]` table of
 * `rust/Cargo.toml`, the key release-please bumps for the CLI (`$.workspace.package.version` in
 * `release-please-config.mjs`). A test that expects the CLI's version reads it from here, so a
 * release pull request, which changes that file, changes what its own tests expect.
 */
export function workspaceVersion(text: string = readFileSync(manifest, 'utf8')): string {
  let table = '';
  for (const line of text.split('\n')) {
    const header = /^\[([^\]]+)\]\s*$/.exec(line);
    if (header) {
      table = header[1];
      continue;
    }
    if (table !== 'workspace.package') continue;
    const version = /^version\s*=\s*"([^"]+)"\s*(?:#.*)?$/.exec(line);
    if (version) return version[1];
  }
  throw new Error('rust/Cargo.toml has no version in its [workspace.package] table.');
}
