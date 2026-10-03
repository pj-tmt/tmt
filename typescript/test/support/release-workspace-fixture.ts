// Minimal real Cargo data for exact-ref release metadata contracts; no external dependencies.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

export function writeReleaseWorkspace(root: string): void {
  const packages = [
    {
      name: 'tmt-cli',
      directory: 'rust/crates/tmt-cli',
      member: 'crates/tmt-cli',
      version: '5.0.0-dev',
    },
    {
      name: 'tmt-squad',
      directory: 'extensions/squad',
      member: '../extensions/squad',
      version: '0.1.0-dev',
    },
  ];
  mkdirSync(join(root, 'rust'), { recursive: true });
  writeFileSync(
    join(root, 'rust/Cargo.toml'),
    `[workspace]\nresolver = "2"\nmembers = [${packages.map((p) => JSON.stringify(p.member)).join(', ')}]\n`
  );
  writeFileSync(
    join(root, 'rust/Cargo.lock'),
    `version = 4\n${packages.map((p) => `\n[[package]]\nname = "${p.name}"\nversion = "${p.version}"\n`).join('')}`
  );
  for (const p of packages) {
    mkdirSync(join(root, p.directory, 'src'), { recursive: true });
    writeFileSync(
      join(root, p.directory, 'Cargo.toml'),
      `[package]\nname = "${p.name}"\nversion = "${p.version}"\nedition = "2021"\n`
    );
    writeFileSync(join(root, p.directory, 'src/main.rs'), 'fn main() {}\n');
  }
}
