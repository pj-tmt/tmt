import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { syntheticAlphaVersion } from '../../scripts/release-versions.mjs';
import { parseComponentMap } from '../../scripts/ci-scope.mjs';
import {
  captureVersionState,
  injectVersion,
  verifyDistArtifact,
  verifyDistVersions,
  verifyVersionState,
  type InjectionMetadata,
} from '../../scripts/release-version-injection.mjs';

const roots: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  syncBuiltinESMExports();
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function fixture(product = 'cli', releaseHerdr = false) {
  const root = mkdtempSync(join(tmpdir(), 'tmt-injection-test-'));
  roots.push(root);
  const files: Record<string, string> = {
    'rust/Cargo.toml':
      '[workspace]\nmembers = []\n[workspace.package]\nversion = "5.0.0-dev" # fixed main version\nedition = "2024"\n',
    'rust/crates/tmt-cli/Cargo.toml': '[package]\nname = "tmt-cli"\nversion.workspace = true\n',
    'rust/crates/tmt-core/Cargo.toml': '[package]\nname = "tmt-core"\nversion.workspace = true\n',
    // #1278's private independent boundary; activation belongs to owner-authorized #1418.
    'rust/crates/tmt-driver-herdr/Cargo.toml':
      '[package]\nname = "tmt-driver-herdr"\nversion = "0.1.0-dev"\n',
    'extensions/ops/Cargo.toml': '[package]\nname = "tmt-ops"\nversion = "0.1.0-dev"\n',
    'rust/Cargo.lock':
      'version = 4\n\n[[package]]\nname = "tmt-cli"\nversion = "5.0.0-dev"\ndependencies = ["tmt-core 5.0.0-dev", "tmt-driver-herdr"]\n\n[[package]]\nname = "tmt-core"\nversion = "5.0.0-dev"\n\n[[package]]\nname = "tmt-driver-herdr"\nversion = "0.1.0-dev"\n\n[[package]]\nname = "tmt-ops"\nversion = "0.1.0-dev"\n\n[[package]]\nname = "external"\nversion = "1.0.0"\nsource = "registry+https://example.test"\nchecksum = "safe"\n',
    'rust/crates/tmt-cli/src/main.rs': 'fn main() {}\n',
  };
  for (const extension of ['remote', 'colab']) {
    files[`extensions/tmt-${extension}/rust/tmt-${extension}/Cargo.toml`] =
      `[package]\nname = "tmt-${extension}"\nversion = "0.1.0-dev"\n`;
    files['rust/Cargo.lock'] += `\n[[package]]\nname = "tmt-${extension}"\nversion = "0.1.0-dev"\n`;
  }
  for (const [file, value] of Object.entries(files)) {
    mkdirSync(dirname(join(root, file)), { recursive: true });
    writeFileSync(join(root, file), value);
  }
  const crates = ['tmt-cli', 'tmt-core', 'tmt-driver-herdr', 'tmt-ops', 'tmt-remote', 'tmt-colab'];
  const metadata: InjectionMetadata = {
    workspace_members: crates,
    packages: crates.map((name) => ({
      id: name,
      name,
      version: name === 'tmt-cli' || name === 'tmt-core' ? '5.0.0-dev' : '0.1.0-dev',
      manifest_path: join(
        root,
        name === 'tmt-ops'
          ? 'extensions/ops/Cargo.toml'
          : ['tmt-remote', 'tmt-colab'].includes(name)
            ? `extensions/${name}/rust/${name}/Cargo.toml`
            : `rust/crates/${name}/Cargo.toml`
      ),
    })),
  };
  const map = parseComponentMap(
    JSON.stringify({
      components: {
        cli: { package: 'tmt-cli', owns: ['.'] },
        ops: { package: 'tmt-ops', owns: ['extensions/ops'] },
        'tmt-remote': { package: 'tmt-remote', owns: ['extensions/tmt-remote'] },
        'tmt-colab': { package: 'tmt-colab', owns: ['extensions/tmt-colab'] },
        'driver-herdr': {
          package: 'tmt-driver-herdr',
          owns: ['rust/crates/tmt-driver-herdr'],
          release: releaseHerdr,
          bootstrapSha: 'a'.repeat(40),
        },
      },
    })
  );
  const tag = product === 'cli' ? 'v5.0.0-alpha.999' : `tmt-${product}-v0.1.0-alpha.999`;
  const snapshot = captureVersionState({
    root,
    files: Object.keys(files),
    metadata,
    product,
    tag,
    cut: 'a'.repeat(40),
    map,
  });
  const resolveMetadata = () => ({
    ...metadata,
    packages: metadata.packages.map((p) => ({
      ...p,
      version: snapshot.packages.includes(p.name) ? snapshot.version : p.version,
    })),
  });
  const updateLock = () =>
    writeFileSync(
      join(root, 'rust/Cargo.lock'),
      product === 'cli'
        ? files['rust/Cargo.lock'].replaceAll('5.0.0-dev', snapshot.version)
        : files['rust/Cargo.lock'].replace(
            `name = "tmt-${product}"\nversion = "0.1.0-dev"`,
            `name = "tmt-${product}"\nversion = "${snapshot.version}"`
          )
    );
  return { root, files: Object.keys(files), snapshot, metadata, resolveMetadata, updateLock, map };
}

describe('mechanical version injection', () => {
  it('gives the TOML helper a bounded 60 seconds for cold Rosetta startup', () => {
    const f = fixture();
    const spawn = vi.spyOn(childProcess, 'spawnSync');
    syncBuiltinESMExports();
    injectVersion(f.root, f.snapshot);
    expect(spawn).toHaveBeenCalledWith(
      expect.stringMatching(/\/debug\/release-version$/),
      ['edit', f.snapshot.section, f.snapshot.oldVersion, f.snapshot.version],
      expect.objectContaining({ stdio: [expect.any(Number), 'pipe', 'pipe'], timeout: 60_000 })
    );
    expect(readFileSync(join(f.root, f.snapshot.manifest), 'utf8')).toBe(
      f.snapshot.source.replace(f.snapshot.oldVersion, f.snapshot.version)
    );
  });
  it('feeds stdin from a file, so an input above the 64 KiB pipe buffer never uses a pipe write', () => {
    const f = fixture();
    // A real Cargo.lock-sized input: more than 64 KiB of packages, parsed by the real helper.
    const filler = Array.from(
      { length: 2500 },
      (_, i) => `\n[[package]]\nname = "filler-${i}"\nversion = "1.0.0"\n`
    ).join('');
    expect(Buffer.byteLength(filler)).toBeGreaterThan(64 * 1024);
    const lockPath = join(f.root, 'rust/Cargo.lock');
    writeFileSync(lockPath, readFileSync(lockPath, 'utf8') + filler);
    const spawn = vi.spyOn(childProcess, 'spawnSync');
    const write = vi.spyOn(process.stderr, 'write').mockReturnValue(true);
    syncBuiltinESMExports();
    const snapshot = captureVersionState({
      root: f.root,
      files: f.files,
      metadata: f.metadata,
      product: 'cli',
      tag: 'v5.0.0-alpha.999',
      cut: 'a'.repeat(40),
      map: f.map,
    });
    expect(snapshot.lock).toContain('filler-2499');
    injectVersion(f.root, snapshot);
    writeFileSync(
      lockPath,
      readFileSync(lockPath, 'utf8').replaceAll(snapshot.oldVersion, snapshot.version)
    );
    // The final verification parses the full-size lock through the helper again.
    verifyVersionState(f.root, snapshot, f.resolveMetadata());
    const sizes = write.mock.calls.map(([line]) =>
      Number(/(\d+) input bytes/.exec(String(line))?.[1])
    );
    expect(Math.max(...sizes)).toBeGreaterThan(64 * 1024);
    expect(spawn.mock.calls.length).toBeGreaterThan(0);
    for (const [, , options] of spawn.mock.calls) {
      expect(options).not.toHaveProperty('input');
      expect((options as { stdio: unknown[] }).stdio[0]).toEqual(expect.any(Number));
    }
  });
  it('logs the duration and input size of every helper call, including a timed-out one', () => {
    const f = fixture();
    const write = vi.spyOn(process.stderr, 'write').mockReturnValue(true);
    injectVersion(f.root, f.snapshot);
    const lines = write.mock.calls.map(([line]) => String(line));
    expect(lines).toEqual([
      expect.stringMatching(
        new RegExp(
          `^release-version edit: ${Buffer.byteLength(f.snapshot.source)} input bytes, \\d+ms, ok\\n$`
        )
      ),
    ]);
    write.mockClear();
    const g = fixture();
    vi.spyOn(childProcess, 'spawnSync').mockReturnValueOnce({
      pid: 0,
      output: [],
      stdout: '',
      stderr: '',
      status: null,
      signal: null,
      error: Object.assign(new Error('spawnSync ETIMEDOUT'), { code: 'ETIMEDOUT' }),
    });
    syncBuiltinESMExports();
    expect(() => injectVersion(g.root, g.snapshot)).toThrow('ETIMEDOUT');
    expect(String(write.mock.calls.at(-1)![0])).toMatch(
      /release-version \w+: \d+ input bytes, \d+ms, failed \(ETIMEDOUT\)/
    );
  });
  it.each(['timeout', 'exit', 'signal'])(
    'keeps the source unchanged and fails loudly on TOML helper %s',
    (failure) => {
      const f = fixture();
      const error = Object.assign(new Error('TOML helper timed out'), { code: 'ETIMEDOUT' });
      vi.spyOn(childProcess, 'spawnSync').mockReturnValueOnce({
        pid: 0,
        output: [],
        stdout: '',
        stderr: 'helper rejected the edit',
        status: failure === 'exit' ? 1 : null,
        signal: failure === 'signal' ? 'SIGTERM' : null,
        ...(failure === 'timeout' ? { error } : {}),
      });
      syncBuiltinESMExports();
      expect(() => injectVersion(f.root, f.snapshot)).toThrow(
        failure === 'timeout' ? error : 'Rust TOML helper failed: helper rejected the edit'
      );
      expect(readFileSync(join(f.root, f.snapshot.manifest), 'utf8')).toBe(f.snapshot.source);
    }
  );
  it.each(['cli', 'ops', 'remote', 'colab'])(
    'injects a synthetic alpha for tagless %s preparation through the unchanged source gate',
    (product) => {
      const f = fixture(product);
      const snapshot = captureVersionState({
        root: f.root,
        files: Object.keys(f.snapshot.hashes),
        metadata: f.metadata,
        product,
        tag: '',
        cut: f.snapshot.cut,
        map: f.map,
      });
      Object.assign(f.snapshot, snapshot);
      expect(snapshot.version).toBe(syntheticAlphaVersion(snapshot.oldVersion));
      injectVersion(f.root, snapshot);
      expect(() => verifyVersionState(f.root, snapshot, f.metadata)).toThrow('implied lock');
      f.updateLock();
      expect(verifyVersionState(f.root, snapshot, f.resolveMetadata()).changed).toEqual(
        ['rust/Cargo.lock', snapshot.manifest].sort()
      );
      writeFileSync(join(f.root, 'rust/crates/tmt-cli/src/main.rs'), 'fn unreviewed() {}\n');
      expect(() => verifyVersionState(f.root, snapshot, f.resolveMetadata())).toThrow(
        'Source differs'
      );
    }
  );
  it.each(['5.0.0-dev', '0.1.0-dev', '5.0.0', '5.0.0-alpha.42'])(
    'uses only the committed core of %s for synthetic preparation',
    (version) => {
      expect(syntheticAlphaVersion(version)).toBe(`${version.split('-')[0]}-alpha.999999`);
    }
  );
  it.each(['unversioned', '5.0', '9'.repeat(400) + '.0.0'])(
    'rejects unusable committed preparation version %s',
    (version) => {
      expect(() => syntheticAlphaVersion(version)).toThrow();
    }
  );
  it('reproves already-versioned historical reruns without rewriting the manifest or lock', () => {
    const f = fixture();
    injectVersion(f.root, f.snapshot);
    f.updateLock();
    const snapshot = captureVersionState({
      root: f.root,
      files: Object.keys(f.snapshot.hashes),
      metadata: f.resolveMetadata(),
      product: 'cli',
      tag: f.snapshot.tag,
      cut: f.snapshot.cut,
      map: f.map,
    });
    const source = readFileSync(join(f.root, snapshot.manifest));
    const lock = readFileSync(join(f.root, 'rust/Cargo.lock'));
    injectVersion(f.root, snapshot);
    expect(verifyVersionState(f.root, snapshot, f.resolveMetadata()).changed).toEqual([]);
    expect(readFileSync(join(f.root, snapshot.manifest))).toEqual(source);
    expect(readFileSync(join(f.root, 'rust/Cargo.lock'))).toEqual(lock);
    writeFileSync(join(f.root, 'rust/crates/tmt-cli/src/main.rs'), 'fn changed() {}\n');
    expect(() => verifyVersionState(f.root, snapshot, f.resolveMetadata())).toThrow(
      'Source differs'
    );
  });
  it.each(['cli', 'ops', 'remote', 'colab'])(
    'injects %s while preserving private independently versioned Herdr',
    (product) => {
      const f = fixture(product);
      injectVersion(f.root, f.snapshot);
      f.updateLock();
      const gate = verifyVersionState(f.root, f.snapshot, f.resolveMetadata());
      expect(gate.packages).toEqual(
        product === 'cli' ? ['tmt-cli', 'tmt-core'] : [`tmt-${product}`]
      );
      expect(
        readFileSync(join(f.root, 'rust/crates/tmt-driver-herdr/Cargo.toml'), 'utf8')
      ).toContain('version = "0.1.0-dev"');
      expect(gate.changed).toEqual(['rust/Cargo.lock', f.snapshot.manifest].sort());
    }
  );
  it('rejects a stale lock and stale resolved package versions', () => {
    const f = fixture();
    injectVersion(f.root, f.snapshot);
    expect(() => verifyVersionState(f.root, f.snapshot, f.metadata)).toThrow('implied lock');
    f.updateLock();
    expect(() => verifyVersionState(f.root, f.snapshot, f.metadata)).toThrow(
      'Resolved product version'
    );
  });
  it('rejects a newly inherited workspace member whose captured lock row is not the dev version', () => {
    const f = fixture();
    const manifest = 'rust/crates/new-member/Cargo.toml';
    mkdirSync(dirname(join(f.root, manifest)), { recursive: true });
    writeFileSync(
      join(f.root, manifest),
      '[package]\nname = "new-member"\nversion.workspace = true\n'
    );
    const lock = `${readFileSync(join(f.root, 'rust/Cargo.lock'), 'utf8')}\n[[package]]\nname = "new-member"\nversion = "5.0.0-alpha.49"\n`;
    writeFileSync(join(f.root, 'rust/Cargo.lock'), lock);
    const metadata = {
      workspace_members: [...f.metadata.workspace_members, 'new-member'],
      packages: [
        ...f.metadata.packages,
        {
          id: 'new-member',
          name: 'new-member',
          version: '5.0.0-dev',
          manifest_path: join(f.root, manifest),
        },
      ],
    };
    const snapshot = captureVersionState({
      root: f.root,
      files: [...Object.keys(f.snapshot.hashes), manifest],
      metadata,
      product: 'cli',
      tag: f.snapshot.tag,
      cut: f.snapshot.cut,
      map: f.map,
    });
    expect(snapshot.packages).toContain('new-member');
    injectVersion(f.root, snapshot);
    writeFileSync(
      join(f.root, 'rust/Cargo.lock'),
      lock.replaceAll('5.0.0-dev', snapshot.version).replace('5.0.0-alpha.49', snapshot.version)
    );
    const resolved = {
      ...metadata,
      packages: metadata.packages.map((p) => ({
        ...p,
        version: snapshot.packages.includes(p.name) ? snapshot.version : p.version,
      })),
    };
    expect(() => verifyVersionState(f.root, snapshot, resolved)).toThrow('5.0.0-alpha.49');
  });
  it.each(['source', 'dependency', 'checksum', 'manifest', 'comment', 'driver'])(
    'rejects unrelated %s changes',
    (change) => {
      const f = fixture();
      injectVersion(f.root, f.snapshot);
      f.updateLock();
      const edit = (file: string, transform: (s: string) => string) =>
        writeFileSync(join(f.root, file), transform(readFileSync(join(f.root, file), 'utf8')));
      if (change === 'source')
        edit('rust/crates/tmt-cli/src/main.rs', (s) => s + '// changed behavior\n');
      if (change === 'dependency')
        edit('rust/Cargo.lock', (s) =>
          s.replace('name = "external"\nversion = "1.0.0"', 'name = "external"\nversion = "1.0.1"')
        );
      if (change === 'checksum')
        edit('rust/Cargo.lock', (s) => s.replace('checksum = "safe"', 'checksum = "evil"'));
      if (change === 'manifest')
        edit('rust/Cargo.toml', (s) => s.replace('edition = "2024"', 'edition = "2021"'));
      if (change === 'comment')
        edit('rust/Cargo.toml', (s) => s.replace('fixed main version', 'unreviewed comment edit'));
      if (change === 'driver')
        edit('rust/crates/tmt-driver-herdr/Cargo.toml', (s) =>
          s.replace('0.1.0-dev', '5.0.0-alpha.999')
        );
      expect(() => verifyVersionState(f.root, f.snapshot, f.resolveMetadata())).toThrow();
    }
  );
  it('refuses a version source changed after capture and an ambiguous declaration', () => {
    const f = fixture();
    writeFileSync(join(f.root, f.snapshot.manifest), f.snapshot.source + '# changed\n');
    expect(() => injectVersion(f.root, f.snapshot)).toThrow('changed before injection');
    f.snapshot.source = f.snapshot.source.replace(
      'version = "5.0.0-dev"',
      'version = "5.0.0-dev"\nversion = "5.0.0-dev"'
    );
    writeFileSync(join(f.root, f.snapshot.manifest), f.snapshot.source);
    expect(() => injectVersion(f.root, f.snapshot)).toThrow('duplicate key');
  });
});

describe('dist and binary version agreement', () => {
  it.each(['cli', 'ops', 'remote', 'colab'])(
    'accepts only %s tag/plan/build/binary agreement',
    (product) => {
      const { snapshot } = fixture(product);
      const manifest = {
        announcement_tag: snapshot.tag,
        releases: [{ app_name: `tmt-${product}`, app_version: snapshot.version }],
      };
      const reported = product === 'cli' ? snapshot.version : `${product} ${snapshot.version}`;
      expect(() => verifyDistVersions(snapshot, manifest, manifest, reported)).not.toThrow();
      for (const wrong of [
        { ...manifest, announcement_tag: 'v9.0.0' },
        { ...manifest, releases: [{ app_name: 'tmt-office', app_version: snapshot.version }] },
        { ...manifest, releases: [{ app_name: `tmt-${product}`, app_version: '5.0.0-dev' }] },
        { ...manifest, releases: [...manifest.releases, ...manifest.releases] },
      ]) {
        expect(() => verifyDistVersions(snapshot, manifest, wrong, reported)).toThrow();
      }
      expect(() => verifyDistVersions(snapshot, manifest, manifest, '5.0.0-dev')).toThrow(
        'binary version'
      );
    }
  );
});

describe('Herdr archive version protocol', () => {
  function artifact(product = 'driver-herdr') {
    const { root, snapshot } = fixture(product, true);
    const manifest = {
      announcement_tag: snapshot.tag,
      releases: [{ app_name: `tmt-${product}`, app_version: snapshot.version }],
    };
    const declaration = { name: 'herdr', kind: 'host', version: snapshot.version, protocols: [1] };
    return { root, snapshot, manifest, declaration };
  }

  it('requires the exact compiled capability version and retains tag/plan/build equality', () => {
    const { snapshot, manifest, declaration } = artifact();
    const raw = JSON.stringify({ ok: declaration });
    expect(() => verifyDistVersions(snapshot, manifest, manifest, raw)).not.toThrow();
    for (const mutation of [
      { name: 'other' },
      { kind: 'provider' },
      { version: '0.1.0-dev' },
      { protocols: [2] },
    ]) {
      expect(() =>
        verifyDistVersions(
          snapshot,
          manifest,
          manifest,
          JSON.stringify({ ok: { ...declaration, ...mutation } })
        )
      ).toThrow('Herdr driver');
    }
    for (const rejected of ['not JSON', '{}', JSON.stringify({ error: { code: 'invalid' } })]) {
      expect(() => verifyDistVersions(snapshot, manifest, manifest, rejected)).toThrow();
    }
    for (const wrong of [
      { ...manifest, announcement_tag: 'tmt-driver-herdr-v9.0.0' },
      { ...manifest, releases: [{ app_name: 'tmt-cli', app_version: snapshot.version }] },
      { ...manifest, releases: [{ app_name: 'tmt-driver-herdr', app_version: '0.1.0-dev' }] },
      { ...manifest, releases: [...manifest.releases, ...manifest.releases] },
    ]) {
      expect(() => verifyDistVersions(snapshot, wrong, manifest, raw)).toThrow();
      expect(() => verifyDistVersions(snapshot, manifest, wrong, raw)).toThrow();
    }
  });

  it.each(['driver-herdr', 'cli', 'ops', 'remote', 'colab'])(
    'uses the existing %s query ABI with unchanged command bounds',
    (product) => {
      const { root, snapshot, manifest, declaration } = artifact(product);
      const stdout =
        product === 'driver-herdr'
          ? JSON.stringify({ ok: declaration })
          : product === 'cli'
            ? snapshot.version
            : `${product} ${snapshot.version}`;
      const spawn = vi.spyOn(childProcess, 'spawnSync').mockReturnValueOnce({
        pid: 0,
        output: [],
        stdout: `${stdout}\n`,
        stderr: '',
        status: 0,
        signal: null,
      });
      syncBuiltinESMExports();
      verifyDistArtifact(root, snapshot, manifest, manifest, '/archive/binary');
      expect(spawn).toHaveBeenCalledExactlyOnceWith(
        '/archive/binary',
        product === 'driver-herdr' ? ['__tmt-driver', '1', 'capabilities'] : ['--version'],
        {
          cwd: root,
          env: process.env,
          encoding: 'utf8',
          timeout: 60_000,
          maxBuffer: 64 * 1024 * 1024,
        }
      );
    }
  );

  it.each(['timeout', 'exit', 'signal'])('refuses an unsuccessful Herdr query: %s', (failure) => {
    const { root, snapshot, manifest, declaration } = artifact();
    vi.spyOn(childProcess, 'spawnSync').mockReturnValueOnce({
      pid: 0,
      output: [],
      stdout: JSON.stringify({ ok: declaration }),
      stderr: '',
      status: failure === 'exit' ? 2 : null,
      signal: failure === 'signal' ? 'SIGTERM' : null,
      ...(failure === 'timeout' ? { error: new Error('ETIMEDOUT') } : {}),
    });
    syncBuiltinESMExports();
    expect(() =>
      verifyDistArtifact(root, snapshot, manifest, manifest, '/archive/binary')
    ).toThrow();
  });
});
