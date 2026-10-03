import { afterEach, describe, expect, it } from 'vite-plus/test';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { parseComponentMap } from '../../scripts/ci-scope.mjs';
import {
  captureVersionState,
  injectVersion,
  verifyDistVersions,
  verifyVersionState,
  type InjectionMetadata,
} from '../../scripts/release-version-injection.mjs';

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function fixture(product = 'cli') {
  const root = mkdtempSync(join(tmpdir(), 'tmt-injection-test-'));
  roots.push(root);
  const files: Record<string, string> = {
    'rust/Cargo.toml':
      '[workspace]\nmembers = []\n[workspace.package]\nversion = "5.0.0-dev" # fixed main version\nedition = "2024"\n',
    'rust/crates/tmt-cli/Cargo.toml': '[package]\nname = "tmt-cli"\nversion.workspace = true\n',
    'rust/crates/tmt-core/Cargo.toml': '[package]\nname = "tmt-core"\nversion.workspace = true\n',
    // #1278's narrowed boundary: private/independently versioned, enabled for release-cut in B.
    'rust/crates/tmt-driver-herdr/Cargo.toml':
      '[package]\nname = "tmt-driver-herdr"\nversion = "0.1.0-dev"\n',
    'extensions/squad/Cargo.toml': '[package]\nname = "tmt-squad"\nversion = "0.1.0-dev"\n',
    'rust/Cargo.lock':
      'version = 4\n\n[[package]]\nname = "tmt-cli"\nversion = "5.0.0-dev"\ndependencies = ["tmt-core 5.0.0-dev", "tmt-driver-herdr"]\n\n[[package]]\nname = "tmt-core"\nversion = "5.0.0-dev"\n\n[[package]]\nname = "tmt-driver-herdr"\nversion = "0.1.0-dev"\n\n[[package]]\nname = "tmt-squad"\nversion = "0.1.0-dev"\n\n[[package]]\nname = "external"\nversion = "1.0.0"\nsource = "registry+https://example.test"\nchecksum = "safe"\n',
    'rust/crates/tmt-cli/src/main.rs': 'fn main() {}\n',
  };
  for (const [file, value] of Object.entries(files)) {
    mkdirSync(dirname(join(root, file)), { recursive: true });
    writeFileSync(join(root, file), value);
  }
  const crates = ['tmt-cli', 'tmt-core', 'tmt-driver-herdr', 'tmt-squad'];
  const metadata: InjectionMetadata = {
    workspace_members: crates,
    packages: crates.map((name) => ({
      id: name,
      name,
      version: name === 'tmt-cli' || name === 'tmt-core' ? '5.0.0-dev' : '0.1.0-dev',
      manifest_path: join(
        root,
        name === 'tmt-squad' ? 'extensions/squad/Cargo.toml' : `rust/crates/${name}/Cargo.toml`
      ),
    })),
  };
  const map = parseComponentMap(
    JSON.stringify({
      components: {
        cli: { package: 'tmt-cli', owns: ['.'] },
        squad: { package: 'tmt-squad', owns: ['extensions/squad'] },
        'driver-herdr': {
          package: 'tmt-driver-herdr',
          owns: ['rust/crates/tmt-driver-herdr'],
          release: false,
          bootstrapSha: 'a'.repeat(40),
        },
      },
    })
  );
  const tag = product === 'cli' ? 'v5.0.0-alpha.999' : 'tmt-squad-v0.1.0-alpha.999';
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
            'name = "tmt-squad"\nversion = "0.1.0-dev"',
            `name = "tmt-squad"\nversion = "${snapshot.version}"`
          )
    );
  return { root, snapshot, metadata, resolveMetadata, updateLock };
}

describe('mechanical version injection', () => {
  it.each(['cli', 'squad'])(
    'injects %s while preserving private independently versioned Herdr',
    (product) => {
      const f = fixture(product);
      injectVersion(f.root, f.snapshot);
      f.updateLock();
      const gate = verifyVersionState(f.root, f.snapshot, f.resolveMetadata());
      expect(gate.packages).toEqual(product === 'cli' ? ['tmt-cli', 'tmt-core'] : ['tmt-squad']);
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
    expect(() => injectVersion(f.root, f.snapshot)).toThrow('Ambiguous');
  });
});

describe('dist and binary version agreement', () => {
  it.each(['cli', 'squad'])('accepts only %s tag/plan/build/binary agreement', (product) => {
    const { snapshot } = fixture(product);
    const manifest = {
      announcement_tag: snapshot.tag,
      releases: [{ app_name: `tmt-${product}`, app_version: snapshot.version }],
    };
    const reported = product === 'cli' ? snapshot.version : `squad ${snapshot.version}`;
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
  });
});
