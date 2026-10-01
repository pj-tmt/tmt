import { existsSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { beforeAll, describe, expect, it } from 'vitest';
import { ownerOf, parseComponentMap } from '../../scripts/ci-scope.mjs';
import {
  generateReleasePleaseConfig,
  readWorkspace,
  renderReleasePleaseConfig,
  type ReleasePleaseConfig,
  type Workspace,
  type WorkspaceCrate,
} from '../../scripts/release-please-config.mjs';

const typescriptRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const root = path.resolve(typescriptRoot, '..');
const { releasePolicy } = (await import(
  pathToFileURL(path.join(typescriptRoot, 'scripts', 'native-release-policy.mjs')).href
)) as { releasePolicy: (product: string) => { tagPrefix: string } };

const read = (file: string) => readFileSync(path.join(root, file), 'utf8');
const readJson = (file: string) => JSON.parse(read(file));

const components = () => parseComponentMap(read('.github/components.json')).components;

const crate = (
  name: string,
  dir: string,
  dependencies: string[] = [],
  inheritsVersion = false,
  hasBinary = true,
  dist?: boolean
): WorkspaceCrate => ({
  name,
  version: '5.0.0-alpha.8',
  manifest: `${dir}/Cargo.toml`,
  dir,
  inheritsVersion,
  hasBinary,
  dist,
  dependencies,
});

/** Three components shaped like the repository: shared core, an extension that links it, a leaf. */
function fixture(): {
  components: ReturnType<typeof components>;
  workspace: { crates: WorkspaceCrate[]; lockNames: Set<string>; files: string[] };
} {
  return {
    components: parseComponentMap(
      JSON.stringify({
        components: {
          cli: {
            package: 'tmt-cli',
            owns: ['.'],
            excludes: ['extensions/tmt-office', 'extensions/tmt-squad'],
          },
          office: { package: 'tmt-office', owns: ['extensions/tmt-office'] },
          squad: { package: 'tmt-squad', owns: ['extensions/tmt-squad'] },
        },
      })
    ).components,
    workspace: {
      crates: [
        crate('core', 'rust/crates/core', [], true),
        crate('cli', 'rust/crates/cli', ['core', 'office-model']),
        crate('office-model', 'extensions/tmt-office/rust/model', ['core'], true),
        crate('office', 'extensions/tmt-office/rust/office', ['office-model', 'core']),
        crate('squad', 'extensions/tmt-squad/rust/squad', ['core']),
      ],
      lockNames: new Set(['core', 'cli', 'office-model', 'office', 'squad']),
      files: [
        'rust/crates/core/src/lib.rs',
        'rust/crates/cli/src/main.rs',
        'extensions/tmt-office/rust/model/src/lib.rs',
        'extensions/tmt-office/rust/office/src/main.rs',
        'extensions/tmt-office/typescript/app.ts',
        'extensions/tmt-office/contracts/a.json',
        'extensions/tmt-squad/rust/squad/src/main.rs',
      ],
    },
  };
}

const generate = (input = fixture()) => generateReleasePleaseConfig(input);

describe('release-please configuration generator', () => {
  it('owns a private browser package without creating a release or hiding an opted-in binary', () => {
    const input = fixture();
    const root = 'extensions/tmt-remote/typescript/browser-addon';
    const privateOwner = parseComponentMap(
      JSON.stringify({ components: { addon: { owns: [root], release: false } } })
    ).components[0];
    input.components = [
      ...input.components.map((c) =>
        c.name === 'cli' ? { ...c, excludes: [...c.excludes, root] } : c
      ),
      privateOwner,
    ];
    input.workspace.files.push(`${root}/src/popup.ts`);
    const generated = generateReleasePleaseConfig(input);
    expect(generated.packages[root]).toBeUndefined();
    expect(generated.packages['.']['exclude-paths']).toContain(root);
    input.workspace.crates.push(crate('hidden-native', `${root}/rust/hidden`));
    expect(() => generateReleasePleaseConfig(input)).toThrow('binary without dist=false');
  });

  it('allows private libraries and opted-out binaries but refuses published or unspecified binaries', () => {
    const input = fixture();
    const root = 'extensions/private/rust';
    const owner = parseComponentMap(
      JSON.stringify({ components: { private: { owns: [root], release: false } } })
    ).components[0];
    input.components = [
      ...input.components.map((c) =>
        c.name === 'cli' ? { ...c, excludes: [...c.excludes, root] } : c
      ),
      owner,
    ];
    input.workspace.crates.push(crate('private-library', `${root}/library`, [], true, false));
    input.workspace.lockNames.add('private-library');
    expect(generate(input).packages[root]).toBeUndefined();
    input.workspace.crates.push(crate('private-binary', `${root}/binary`, [], false, true, false));
    input.workspace.lockNames.add('private-binary');
    const generated = generate(input);
    expect(generated.packages[root]).toBeUndefined();
    expect(generated.packages['.']['exclude-paths']).toContain(root);
    expect(JSON.stringify(generated)).not.toContain('private-library');
    expect(JSON.stringify(generated)).not.toContain('private-binary');
    for (const dist of [true, undefined]) {
      input.workspace.crates[input.workspace.crates.length - 1] = crate(
        'private-binary',
        `${root}/binary`,
        [],
        false,
        true,
        dist
      );
      expect(() => generate(input)).toThrow('binary without dist=false');
    }
  });

  it('makes one package per component with the tag the publication policy expects', () => {
    const { packages } = generate();
    expect(Object.keys(packages)).toEqual(['.', 'extensions/tmt-office', 'extensions/tmt-squad']);
    expect(packages['.']).toMatchObject({
      component: 'tmt-cli',
      'include-component-in-tag': false,
    });
    expect(packages['extensions/tmt-office']).toMatchObject({
      component: 'tmt-office',
      'include-component-in-tag': true,
    });
    for (const [product, packagePath] of [
      ['cli', '.'],
      ['office', 'extensions/tmt-office'],
      ['squad', 'extensions/tmt-squad'],
    ]) {
      const policy = releasePolicy(product);
      const config = packages[packagePath];
      const prefix = config['include-component-in-tag'] ? `${config.component}-v` : 'v';
      expect(prefix, product).toBe(policy.tagPrefix);
    }
  });

  it('rebuilds every open release pull request on each run, so a conflict with main clears', () => {
    expect(generate()).toMatchObject({ 'always-update': true });
  });

  it('keeps every package in the alpha line: a false prerelease would graduate it to a stable version', () => {
    const config = generate();
    expect(config).toMatchObject({ prerelease: true, 'prerelease-type': 'alpha' });
    for (const entry of Object.values(config.packages))
      expect(entry).not.toHaveProperty('prerelease');
  });

  it('updates the workspace version once, in the package that owns the workspace manifest', () => {
    const { packages } = generate();
    const workspaceVersions = Object.entries(packages).flatMap(([packagePath, config]) =>
      config['extra-files']
        .filter((file) => file.jsonpath === '$.workspace.package.version')
        .map((file) => `${packagePath}: ${file.path}`)
    );
    expect(workspaceVersions).toEqual(['.: rust/Cargo.toml']);
  });

  it('gives a crate that declares its own version to the component that owns the crate', () => {
    const { packages } = generate();
    expect(packages['extensions/tmt-office']['extra-files']).toContainEqual({
      type: 'toml',
      path: 'rust/office/Cargo.toml',
      jsonpath: '$.package.version',
    });
    expect(packages['.']['extra-files']).toContainEqual({
      type: 'toml',
      path: 'rust/crates/cli/Cargo.toml',
      jsonpath: '$.package.version',
    });
  });

  it('gives each lock entry to whoever declares the crate version, anchored at the root for extensions', () => {
    const { packages } = generate();
    const lockEntries = (packagePath: string) =>
      packages[packagePath]['extra-files']
        .filter((file) => file.path.endsWith('Cargo.lock'))
        .map((file) => `${file.path} ${file.jsonpath.match(/=='([^']+)'/)?.[1]}`);
    // office-model inherits the workspace version, so the CLI release changes its lock entry.
    expect(lockEntries('.')).toEqual([
      'rust/Cargo.lock cli',
      'rust/Cargo.lock core',
      'rust/Cargo.lock office-model',
    ]);
    expect(lockEntries('extensions/tmt-office')).toEqual(['/rust/Cargo.lock office']);
    expect(lockEntries('extensions/tmt-squad')).toEqual(['/rust/Cargo.lock squad']);
  });

  it('keeps the extension crates the CLI links and excludes the rest of each extension root', () => {
    const { packages } = generate();
    // release-please can only drop paths, so the unlinked children of the Office root are listed.
    expect(packages['.']['exclude-paths']).toEqual([
      'extensions/tmt-office/contracts',
      'extensions/tmt-office/rust/office',
      'extensions/tmt-office/typescript',
      'extensions/tmt-squad',
    ]);
    // An extension does not react to changes in the core crates it links: the option does not exist.
    for (const extension of ['extensions/tmt-office', 'extensions/tmt-squad']) {
      expect(packages[extension]).not.toHaveProperty('exclude-paths');
      expect(packages[extension]).not.toHaveProperty('additional-paths');
    }
  });

  it('follows links through other crates but not through dev-dependencies', () => {
    const input = fixture();
    input.workspace.crates.push(crate('office-util', 'extensions/tmt-office/rust/util', [], true));
    input.workspace.crates[2] = crate(
      'office-model',
      'extensions/tmt-office/rust/model',
      ['core', 'office-util'],
      true
    );
    input.workspace.lockNames.add('office-util');
    input.workspace.files.push('extensions/tmt-office/rust/util/src/lib.rs');
    expect(generate(input).packages['.']['exclude-paths']).toEqual([
      'extensions/tmt-office/contracts',
      'extensions/tmt-office/rust/office',
      'extensions/tmt-office/typescript',
      'extensions/tmt-squad',
    ]);
    // A link the reader dropped as a dev-dependency never reaches the config: the whole root goes.
    input.workspace.crates[1] = crate('cli', 'rust/crates/cli', ['core']);
    expect(generate(input).packages['.']['exclude-paths']).toEqual([
      'extensions/tmt-office',
      'extensions/tmt-squad',
    ]);
  });

  it('refuses a tracked file beside a linked crate, which a prefix cannot exclude', () => {
    const input = fixture();
    input.workspace.files.push('extensions/tmt-office/rust/README.md');
    expect(() => generate(input)).toThrow('cannot be excluded by prefix');
  });

  it('refuses a crate with no lock entry rather than leaving its lock line unmanaged', () => {
    const input = fixture();
    input.workspace.lockNames.delete('squad');
    expect(() => generate(input)).toThrow('rust/Cargo.lock has no entry for squad.');
  });

  it('refuses a crate name it cannot write into a JSONPath filter', () => {
    const input = fixture();
    input.workspace.crates.push(crate("bad'name", 'rust/crates/bad', [], true));
    input.workspace.lockNames.add("bad'name");
    expect(() => generate(input)).toThrow('cannot be written into a JSONPath filter');
  });

  it('refuses a component that is not a single package root or has no publication policy', () => {
    const two = fixture();
    two.components = parseComponentMap(
      JSON.stringify({
        components: { cli: { package: 'tmt-cli', owns: ['.', 'docs'] } },
      })
    ).components;
    expect(() => generate(two)).toThrow('must own exactly one root');
    const unknown = fixture();
    unknown.components = parseComponentMap(
      JSON.stringify({ components: { relay: { package: 'tmt-relay', owns: ['relay'] } } })
    ).components;
    expect(() => generate(unknown)).toThrow('Unknown native product: relay');
  });

  it('renders stable JSON that ends with a newline', () => {
    const text = renderReleasePleaseConfig(fixture());
    expect(text.endsWith('}\n')).toBe(true);
    expect(JSON.parse(text)).toEqual(generate());
  });
});

describe('committed release-please configuration', () => {
  let workspace: Workspace;
  const config = readJson('release-please-config.json') as ReleasePleaseConfig;

  beforeAll(() => {
    workspace = readWorkspace(`${root}/`);
  }, 30_000);

  const releasedCrates = () => {
    const map = parseComponentMap(read('.github/components.json'));
    return workspace.crates.filter(
      (crate) =>
        map.components.find((component) => component.name === ownerOf(crate.manifest, map))
          ?.release !== false
    );
  };

  const resolveFromPackage = (packagePath: string, file: string) =>
    file.startsWith('/') ? file.slice(1) : packagePath === '.' ? file : `${packagePath}/${file}`;
  const lockedName = (jsonpath: string) =>
    jsonpath.match(/^\$\.package\[\?\(@\.name\.value=='([^']+)'\)\]\.version$/)?.[1];

  it('is what the generator writes for the component map and the workspace', () => {
    expect(read('release-please-config.json')).toBe(
      renderReleasePleaseConfig({ components: components(), workspace })
    );
  });

  it('reads binary opt-out metadata and excludes the private remote versions', () => {
    expect(workspace.crates.find(({ name }) => name === 'tmt-remote')).toMatchObject({
      hasBinary: true,
      dist: false,
    });
    expect(workspace.crates.find(({ name }) => name === 'tmt-core')).toMatchObject({
      hasBinary: false,
    });
    expect(JSON.stringify(config)).not.toContain('tmt-remote/Cargo.toml');
    expect(JSON.stringify(config)).not.toContain("name.value=='tmt-remote'");
  });

  it('manages every released workspace crate lock entry exactly once, and only real ones', () => {
    const managed = Object.values(config.packages).flatMap((entry) =>
      entry['extra-files'].flatMap((file) => lockedName(file.jsonpath) ?? [])
    );
    expect([...managed].sort()).toEqual(
      releasedCrates()
        .map(({ name }) => name)
        .sort()
    );
  });

  it('manages every released crate version and the workspace version exactly once', () => {
    const managed = Object.entries(config.packages).flatMap(([packagePath, entry]) =>
      entry['extra-files']
        .filter((file) => !file.path.endsWith('Cargo.lock'))
        .map((file) => `${resolveFromPackage(packagePath, file.path)} ${file.jsonpath}`)
    );
    const declared = [
      'rust/Cargo.toml $.workspace.package.version',
      ...releasedCrates()
        .filter(({ inheritsVersion }) => !inheritsVersion)
        .map(({ manifest }) => `${manifest} $.package.version`),
    ];
    expect([...managed].sort()).toEqual([...declared].sort());
  });

  it('points every file and exclude path at something that exists', () => {
    for (const [packagePath, entry] of Object.entries(config.packages)) {
      for (const file of entry['extra-files']) {
        const resolved = resolveFromPackage(packagePath, file.path);
        expect(existsSync(path.join(root, resolved)), `${packagePath}: ${resolved}`).toBe(true);
      }
      for (const excluded of entry['exclude-paths'] ?? []) {
        expect(statSync(path.join(root, excluded)).isDirectory(), excluded).toBe(true);
      }
    }
  });

  it('counts every tracked file under an extension for the CLI exactly when a crate the CLI links owns it', () => {
    const linked = new Set<string>();
    const byName = new Map(workspace.crates.map((c) => [c.name, c]));
    const visit = (name: string) => {
      if (linked.has(name)) return;
      linked.add(name);
      byName.get(name)?.dependencies.forEach(visit);
    };
    workspace.crates
      .filter(({ dir }) => !dir.startsWith('extensions/'))
      .forEach(({ name }) => visit(name));
    const linkedDirs = [...linked].map((name) => byName.get(name)?.dir as string);
    const excluded = config.packages['.']['exclude-paths'] ?? [];
    const under = (file: string, dirs: readonly string[]) =>
      dirs.some((dir) => file.startsWith(`${dir}/`));
    for (const file of workspace.files.filter((f) => f.startsWith('extensions/'))) {
      // Counted for the CLI when it links the owning crate, and never both or neither.
      expect(under(file, linkedDirs) !== under(file, excluded), file).toBe(true);
    }
  });

  it('creates drafts, one pull request per component, and the tags the release policy publishes', () => {
    expect(config).toMatchObject({
      draft: true,
      // Without it an open release pull request that conflicts with main is never rewritten.
      'always-update': true,
      'separate-pull-requests': true,
      'include-v-in-tag': true,
      versioning: 'prerelease',
      'prerelease-type': 'alpha',
      // Also the version line: without it the CLI would graduate from 5.0.0-alpha.8 to 5.0.0.
      prerelease: true,
    });
    const products = {
      '.': 'cli',
      'extensions/tmt-office': 'office',
      'extensions/tmt-squad': 'squad',
    };
    expect(Object.keys(config.packages)).toEqual(Object.keys(products));
    for (const [packagePath, product] of Object.entries(products)) {
      const entry = config.packages[packagePath];
      const policy = releasePolicy(product);
      expect(entry['include-component-in-tag'] ? `${entry.component}-v` : 'v').toBe(
        policy.tagPrefix
      );
      expect(entry).not.toHaveProperty('prerelease');
    }
  });

  it('starts from the last published versions, never ahead of what the crates declare', () => {
    const manifest = readJson('.release-please-manifest.json') as Record<string, string>;
    expect(Object.keys(manifest)).toEqual(Object.keys(config.packages));
    const order = (version: string) => {
      const match = version.match(/^(\d+)\.(\d+)\.(\d+)(?:-alpha\.(\d+))?$/);
      expect(match, `${version} is a version this guard understands`).not.toBeNull();
      const [, major, minor, patch, alpha] = match as RegExpMatchArray;
      return [
        Number(major),
        Number(minor),
        Number(patch),
        alpha === undefined ? Number.MAX_SAFE_INTEGER : Number(alpha),
      ];
    };
    const declared = (name: string) => workspace.crates.find((c) => c.name === name)?.version;
    for (const [packagePath, crateName] of [
      ['.', 'tmt-cli'],
      ['extensions/tmt-office', 'tmt-office'],
      ['extensions/tmt-squad', 'tmt-squad'],
    ]) {
      const [published, declaredNow] = [manifest[packagePath], declared(crateName) as string];
      const [left, right] = [order(published), order(declaredNow)];
      const difference = left.map((part, index) => part - right[index]).find((part) => part !== 0);
      expect(
        difference ?? 0,
        `${packagePath}: ${published} against ${declaredNow}`
      ).toBeLessThanOrEqual(0);
    }
  });
});

describe('pinned release-please CLI', () => {
  const pinned = readJson('.github/release-please/package.json');
  const lock = read('.github/release-please/pnpm-lock.yaml');

  it('is one exact version, locked with an integrity hash for every package', () => {
    expect(pinned.dependencies).toEqual({
      'release-please': expect.stringMatching(/^\d+\.\d+\.\d+$/),
    });
    expect(pinned.private).toBe(true);
    const version = pinned.dependencies['release-please'];
    expect(lock).toContain(`release-please@${version}:`);
    const resolutions = lock.split('\n').filter((line) => line.trim().startsWith('resolution:'));
    expect(resolutions.length).toBeGreaterThan(50);
    for (const line of resolutions)
      expect(line, line).toMatch(/resolution: \{integrity: sha512-[A-Za-z0-9+/=]+\}$/);
  });
});
