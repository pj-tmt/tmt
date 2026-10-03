import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { afterEach, describe, expect, it } from 'vite-plus/test';

const roots: string[] = [];

function tool(directory: string, name: string, source: string) {
  const target = path.join(directory, name);
  writeExecutable(target, `#!/bin/sh\nset -eu\n${source}\n`, 0o700);
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('native artifact stdout', () => {
  it.each(['cli', 'office', 'driver-herdr'])(
    'reserves stdout and selects the %s notice manifest',
    (product) => {
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
          : product === 'driver-herdr'
            ? 'crates/tmt-driver-herdr/Cargo.toml'
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
if [ '${product}' = cli ] && [ "$3" = crates/tmt-driver-herdr/Cargo.toml ]; then :; else test "$3" = '${manifest}'; fi
test -f "$3"
for argument do output=$argument; done
printf 'Rust license notice\\n' > "$output"
printf 'notice diagnostics\\n'
fi`
      );
      const driverManifest = path.join(root, 'rust/crates/tmt-driver-herdr/Cargo.toml');
      mkdirSync(path.dirname(driverManifest), { recursive: true });
      writeFileSync(driverManifest, '# Companion package fixture\n');
      tool(
        bin,
        'cargo',
        `if [ "$1" = pkgid ]; then printf 'path+file:///fixture#tmt-${product}@0.1.0-alpha.2\\n'; else
test "$1 $2 $3 $4 $5 $6" = 'build --locked -p tmt-driver-herdr --bin tmt-driver-herdr'
mkdir -p target/aarch64-apple-darwin/dist
printf 'driver package bytes\\n' > target/aarch64-apple-darwin/dist/tmt-driver-herdr
chmod +x target/aarch64-apple-darwin/dist/tmt-driver-herdr
printf 'companion build diagnostics\\n'
fi`
      );
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
      if (product === 'cli') {
        expect(result.stderr).toContain('companion build diagnostics');
        const companion = path.join(root, 'rust/target/native-companion/tmt-driver-herdr');
        expect(readFileSync(companion, 'utf8')).toBe('driver package bytes\n');
        expect(statSync(companion).mode & 0o111).not.toBe(0);
      } else expect(result.stderr).not.toContain('companion build diagnostics');
      expect(
        readFileSync(path.join(root, 'rust/target/native-notices/THIRD-PARTY-NOTICES.txt'), 'utf8')
      ).toBe(
        `Rust license notice\n${product === 'office' ? 'SPA license notice\n' : product === 'cli' ? 'Rust license notice\n' : ''}`
      );
    }
  );
});
