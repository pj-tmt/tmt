import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { afterEach, describe, expect, it } from 'vitest';

const roots: string[] = [];

function tool(directory: string, name: string, source: string) {
  const target = path.join(directory, name);
  writeExecutable(target, `#!/bin/sh\nset -eu\n${source}\n`, 0o700);
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('native artifact stdout', () => {
  it.each(['cli', 'office'])('reserves stdout and selects the %s notice manifest', (product) => {
    const root = mkdtempSync(path.join(os.tmpdir(), 'tmt-native-stdout-'));
    roots.push(root);
    const bin = path.join(root, 'bin');
    mkdirSync(bin);
    mkdirSync(path.join(root, 'scripts'));
    mkdirSync(path.join(root, 'rust'));
    mkdirSync(path.join(root, 'typescript'));
    const manifest =
      product === 'office'
        ? '../extensions/tmt-office/rust/tmt-office/Cargo.toml'
        : 'crates/tmt-cli/Cargo.toml';
    const manifestPath = path.resolve(root, 'rust', manifest);
    mkdirSync(path.dirname(manifestPath), { recursive: true });
    writeFileSync(manifestPath, '# Selected package fixture\n');
    const script = path.join(root, 'scripts/build-native-artifact.sh');
    writeExecutable(
      script,
      readFileSync(path.resolve('../scripts/build-native-artifact.sh')),
      statSync(path.resolve('../scripts/build-native-artifact.sh')).mode & 0o777
    );
    tool(
      bin,
      'corepack',
      `mkdir -p ../target/office-spa
printf 'SPA license notice\\n' > ../target/office-spa/THIRD-PARTY-NOTICES.txt
printf 'vite diagnostics\\n'`
    );
    tool(bin, 'rustup', `printf '1.97.0-aarch64-apple-darwin (default)\\n'`);
    tool(
      bin,
      'cargo-about',
      `if [ "\${1:-}" = --version ]; then printf 'cargo-about 0.9.2\\n'; else
test "$2" = --manifest-path
test "$3" = '${manifest}'
test -f "$3"
printf 'Rust license notice\\n' > target/native-notices/THIRD-PARTY-NOTICES.txt
printf 'notice diagnostics\\n'
fi`
    );
    tool(bin, 'cargo', `printf 'path+file:///fixture#tmt-office@0.1.0-alpha.2\\n'`);
    tool(
      bin,
      'dist',
      `if [ "\${1:-}" = build ]; then printf '{"artifacts":{}}\\n'; else printf 'dist diagnostics\\n'; fi`
    );
    const result = spawnSync(script, ['aarch64-apple-darwin', product], {
      encoding: 'utf8',
      env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
    });
    expect(result.status, result.stderr).toBe(0);
    expect(JSON.parse(result.stdout)).toEqual({ artifacts: {} });
    if (product === 'office') expect(result.stderr).toContain('vite diagnostics');
    else expect(result.stderr).not.toContain('vite diagnostics');
    expect(result.stderr).toContain('notice diagnostics');
    expect(result.stderr).toContain('dist diagnostics');
    expect(
      readFileSync(path.join(root, 'rust/target/native-notices/THIRD-PARTY-NOTICES.txt'), 'utf8')
    ).toBe(`Rust license notice\n${product === 'office' ? 'SPA license notice\n' : ''}`);
  });
});
