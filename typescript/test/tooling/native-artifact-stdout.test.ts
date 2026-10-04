import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { afterEach, describe, expect, it } from 'vite-plus/test';

const roots: string[] = [];
/** Vendored license clarifications: crate, locked version, vendored file. */
const vendored = [
  ['taffy', '0.7.7', 'LICENSE.md'],
  ['yrs', '0.28.0', 'LICENSE'],
] as const;

function tool(directory: string, name: string, source: string) {
  const target = path.join(directory, name);
  writeExecutable(target, `#!/bin/sh\nset -eu\n${source}\n`, 0o700);
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function artifactFixture(product: string, omitted = '') {
  const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'tmt-native-stdout-&|"\\-')));
  roots.push(root);
  const bin = path.join(root, 'bin');
  mkdirSync(bin);
  mkdirSync(path.join(root, 'scripts'));
  mkdirSync(path.join(root, 'rust'));
  mkdirSync(path.join(root, 'typescript'));
  for (const [crate, version] of vendored)
    mkdirSync(path.join(root, `rust/licenses/${crate}-${version}`), { recursive: true });
  for (const file of [
    'rust/about.toml',
    'rust/Cargo.lock',
    ...vendored.map(([crate, version, name]) => `rust/licenses/${crate}-${version}/${name}`),
  ]) {
    writeFileSync(path.join(root, file), readFileSync(path.resolve('..', file)));
  }
  const manifest =
    product === 'cli'
      ? 'crates/tmt-cli/Cargo.toml'
      : product === 'driver-herdr'
        ? 'crates/tmt-driver-herdr/Cargo.toml'
        : `../extensions/tmt-${product}/rust/tmt-${product}/Cargo.toml`;
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
    `if [ '${product}' = colab ]; then
 test "$1" = pnpm@10.33.0
 if [ "$2" = install ]; then test "$3" = --frozen-lockfile; test "$4" = --ignore-scripts; exit 0; fi
 test "$2" = --filter; test "$3" = @tmt/colab-app; test "$4" = --fail-if-no-match; test "$5" = build
 app=../extensions/tmt-colab/typescript/app/dist
else app=../target/office-spa; fi
mkdir -p "$app/assets"
printf 'index app\\n' > "$app/index.html"
printf 'SPA license notice\\n' > "$app/THIRD-PARTY-NOTICES.txt"
if [ '${omitted}' = index ]; then rm "$app/index.html"; fi
if [ '${omitted}' = notices ]; then : > "$app/THIRD-PARTY-NOTICES.txt"; fi
if [ '${omitted}' = build ]; then exit 9; fi
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
test "$4" = --config
test "$5" = target/native-notices/about.toml
test -f "$5"
case "$*" in *'--locked --offline --fail'*) ;; *) exit 1 ;; esac
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
    `if [ '${product}' = colab ]; then test "$TMT_NATIVE_PRODUCT" = colab; test "$TMT_COLAB_APP_DIR" = '${root}/extensions/tmt-colab/typescript/app/dist'; test -s "$TMT_COLAB_APP_DIR/index.html"; fi
if [ "$1" = pkgid ]; then printf 'path+file:///fixture#tmt-${product}@0.1.0-alpha.2\\n'; else
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
  return { root, bin, script };
}

describe('native artifact stdout', () => {
  it.each(['cli', 'office', 'squad', 'driver-herdr', 'remote', 'colab'])(
    'reserves stdout and selects the %s notice manifest',
    (product) => {
      const { root, bin, script } = artifactFixture(product);
      const result = spawnSync(script, ['aarch64-apple-darwin', product], {
        encoding: 'utf8',
        env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
      });
      expect(result.status, result.stderr).toBe(0);
      expect(JSON.parse(result.stdout)).toEqual({ artifacts: {} });
      if (['office', 'colab'].includes(product))
        expect(result.stderr).toContain('vite diagnostics');
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
        `Rust license notice\n${['office', 'colab'].includes(product) ? 'SPA license notice\n' : product === 'cli' ? 'Rust license notice\n' : ''}`
      );
      const config = readFileSync(path.join(root, 'rust/target/native-notices/about.toml'), 'utf8');
      expect(config).toBe(
        vendored.reduce(
          (text, [crate, version, name]) =>
            text.replace(
              `"__TMT_${crate.toUpperCase()}_LICENSE__"`,
              JSON.stringify(path.join(root, `rust/licenses/${crate}-${version}/${name}`))
            ),
          readFileSync(path.join(root, 'rust/about.toml'), 'utf8')
        )
      );
      expect(config).not.toMatch(/"__TMT_[A-Z]+_LICENSE__"/);
    }
  );
  it.each(['cli', 'office', 'squad', 'driver-herdr', 'remote', 'colab'])(
    'generates %s notices without building the companion or invoking cargo-dist',
    (product) => {
      const { bin, script } = artifactFixture(product);
      tool(bin, 'dist', 'exit 97');
      tool(bin, 'cargo', 'exit 96');
      const noticeOnly = spawnSync(script, ['--notices-only', 'aarch64-apple-darwin', product], {
        encoding: 'utf8',
        env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
      });
      expect(noticeOnly.status, noticeOnly.stderr).toBe(0);
      expect(noticeOnly.stdout).toBe('');
      expect(noticeOnly.stderr).not.toContain('companion build diagnostics');
    }
  );
  it.each(
    vendored.flatMap(([crate, version, name]) =>
      ['cli', 'office', 'squad', 'driver-herdr', 'remote', 'colab'].map(
        (product) => [crate, version, name, product] as const
      )
    )
  )(
    'rejects a corrupted %s license before generating %s notices',
    (crate, version, name, product) => {
      const { root, bin, script } = artifactFixture(product);
      tool(
        bin,
        'cargo-about',
        `if [ "\${1:-}" = --version ]; then printf 'cargo-about 0.9.2\\n'; else exit 98; fi`
      );
      const license = path.join(root, `rust/licenses/${crate}-${version}/${name}`);
      writeFileSync(license, 'corrupted license');
      const corrupted = spawnSync(script, ['--notices-only', 'aarch64-apple-darwin', product], {
        encoding: 'utf8',
        env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
      });
      expect(corrupted.status).toBe(1);
      expect(corrupted.stderr).toContain(`Vendored ${crate} license checksum mismatch`);
    }
  );
  it.each(
    vendored.flatMap(([crate, version]) =>
      ['cli', 'office', 'squad', 'driver-herdr', 'remote', 'colab'].map(
        (product) => [crate, version, product] as const
      )
    )
  )('rejects %s version drift before generating %s notices', (crate, version, product) => {
    const { root, bin, script } = artifactFixture(product);
    tool(
      bin,
      'cargo-about',
      `if [ "\${1:-}" = --version ]; then printf 'cargo-about 0.9.2\\n'; else exit 98; fi`
    );
    const lock = path.join(root, 'rust/Cargo.lock');
    writeFileSync(
      lock,
      readFileSync(lock, 'utf8').replace(
        `name = "${crate}"\nversion = "${version}"`,
        `name = "${crate}"\nversion = "${version}-drift"`
      )
    );
    const upgraded = spawnSync(script, ['--notices-only', 'aarch64-apple-darwin', product], {
      encoding: 'utf8',
      env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
    });
    expect(upgraded.status).toBe(1);
    expect(upgraded.stderr).toContain(`Review ${crate} notice on version changes`);
  });
  it.each(['index', 'notices', 'build'])(
    'rejects Colab packaging with missing or failed %s frontend input',
    (omitted) => {
      const { bin, script } = artifactFixture('colab', omitted);
      const result = spawnSync(script, ['aarch64-apple-darwin', 'colab'], {
        encoding: 'utf8',
        env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
      });
      expect(result.status).toBe(omitted === 'build' ? 9 : 1);
      expect(result.stdout).toBe('');
      expect(result.stderr).not.toContain('notice diagnostics');
      expect(result.stderr).not.toContain('dist diagnostics');
    }
  );
});
