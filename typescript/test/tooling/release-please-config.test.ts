import { existsSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { Ajv } from 'ajv';
import {
  attributeReleaseConsumption,
  assertReleasePleaseApi,
  executeReleasePlease,
  loadPinnedReleasePlease,
  holdTaglessDraftCandidates,
  preserveUnchangedReleasePullRequests,
} from '../../scripts/release-please-run.mjs';
import { ownerOf, parseComponentMap } from '../../scripts/ci-scope.mjs';
import {
  generateReleasePleaseConfig,
  readWorkspace,
  releaseConsumption,
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
const releasePlease = loadPinnedReleasePlease();

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

const lockEntries = (config: ReleasePleaseConfig, packagePath: string) =>
  config.packages[packagePath]['extra-files']
    .filter((file) => file.path.endsWith('Cargo.lock'))
    .map((file) => `${file.path} ${file.jsonpath.match(/=='([^']+)'/)?.[1]}`);

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
    // The library inherits the workspace version, so the CLI release still updates its lock entry;
    // the binary declares its own version and nobody releases it.
    expect(lockEntries(generated, '.')).toContain('rust/Cargo.lock private-library');
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

  it('releases nothing for a parked extension but keeps the lock entries of its inherited versions', () => {
    const input = fixture();
    input.components = input.components.map((c) =>
      c.name === 'office' ? { ...c, release: false } : c
    );
    input.workspace.crates[2] = crate(
      'office-model',
      'extensions/tmt-office/rust/model',
      ['core'],
      true,
      false
    );
    input.workspace.crates[3] = crate(
      'office',
      'extensions/tmt-office/rust/office',
      ['office-model', 'core'],
      false,
      true,
      false
    );
    const generated = generate(input);
    expect(Object.keys(generated.packages)).toEqual(['.', 'extensions/tmt-squad']);
    // office-model inherits the workspace version, so a CLI release must still update its lock
    // entry or the next `cargo --locked` fails; the office binary declares its own version.
    expect(lockEntries(generated, '.')).toEqual([
      'rust/Cargo.lock cli',
      'rust/Cargo.lock core',
      'rust/Cargo.lock office-model',
    ]);
    expect(JSON.stringify(generated)).not.toContain("name.value=='office'");
    expect(JSON.stringify(generated)).not.toContain('rust/office/Cargo.toml');
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
    expect(config).toMatchObject({ prerelease: true });
    for (const entry of Object.values(config.packages)) {
      expect(entry).not.toHaveProperty('prerelease');
      expect(entry['prerelease-type']).toBe('alpha');
    }
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
    const config = generate();
    // office-model inherits the workspace version, so the CLI release changes its lock entry.
    expect(lockEntries(config, '.')).toEqual([
      'rust/Cargo.lock cli',
      'rust/Cargo.lock core',
      'rust/Cargo.lock office-model',
    ]);
    expect(lockEntries(config, 'extensions/tmt-office')).toEqual(['/rust/Cargo.lock office']);
    expect(lockEntries(config, 'extensions/tmt-squad')).toEqual(['/rust/Cargo.lock squad']);
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

  // A crate that inherits the workspace version is released by the owner of rust/Cargo.toml.
  const releasedCrates = () => {
    const map = parseComponentMap(read('.github/components.json'));
    const workspaceOwner = ownerOf('rust/Cargo.toml', map);
    return workspace.crates.filter(
      (crate) =>
        map.components.find(
          (component) =>
            component.name ===
            (crate.inheritsVersion ? workspaceOwner : ownerOf(crate.manifest, map))
        )?.release !== false
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
      // Also the version line: without it the CLI would graduate from 5.0.0-alpha.8 to 5.0.0.
      prerelease: true,
    });
    const products = {
      '.': 'cli',
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

  it('loads the release job pin for both the wrapper and real Manifest tests', () => {
    expect(releasePlease.VERSION).toBe(pinned.dependencies['release-please']);
    expect(readJson('typescript/package.json').devDependencies).not.toHaveProperty(
      'release-please'
    );
    expect(() => assertReleasePleaseApi(releasePlease)).not.toThrow();
    for (const broken of [
      { ...releasePlease, VERSION: '18.0.0' },
      { ...releasePlease, GitHub: { create() {}, prototype: {} } },
      { ...releasePlease, GitHub: { create() {}, prototype: { mergeCommitIterator() {} } } },
    ])
      expect(() => assertReleasePleaseApi(broken)).toThrow('Unsupported release-please API');
  });

  it('passes the pinned config schema, which has no additional-paths option', () => {
    const validate = new Ajv({ strict: false, validateFormats: false }).compile(
      releasePlease.configSchema
    );
    expect(validate(readJson('release-please-config.json')), JSON.stringify(validate.errors)).toBe(
      true
    );
    expect(
      releasePlease.configSchema.definitions.ReleaserConfigOptions.properties
    ).not.toHaveProperty('additional-paths');
  });

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

describe('private leaf release attribution with pinned release-please', () => {
  const squadPath = 'extensions/tmt-squad';
  const leafPath = 'rust/crates/tmt-tui';
  const commit = (sha: string, files: string[], message = 'fix: correct bound text') => ({
    sha,
    files,
    message,
  });

  it('keeps source ownership private and validates declared consumers before running', () => {
    const map = components();
    expect(releaseConsumption(map)).toEqual([{ source: leafPath, target: squadPath }]);
    expect(
      ownerOf(`${leafPath}/src/binding.rs`, parseComponentMap(read('.github/components.json')))
    ).toBe('tmt-tui');
    for (const changed of [
      map.map((c) => (c.name === 'tmt-tui' ? { ...c, release: true } : c)),
      map.map((c) => (c.name === 'tmt-tui' ? { ...c, releaseConsumers: ['missing'] } : c)),
      map.map((c) => (c.name === 'squad' ? { ...c, release: false } : c)),
      map.map((c) => (c.name === 'tmt-tui' ? { ...c, releaseConsumers: ['cli'] } : c)),
    ])
      expect(() => releaseConsumption(changed)).toThrow();
  });

  it('plans both commands in dry mode without calling mutation methods', async () => {
    const calls: string[] = [];
    const manifest = {
      async buildPullRequests() {
        calls.push('plan-pr');
        return [];
      },
      async buildReleases() {
        calls.push('plan-release');
        return [];
      },
      async createPullRequests() {
        calls.push('write-pr');
        return [];
      },
      async createReleases() {
        calls.push('write-release');
        return [];
      },
    };
    for (const command of ['release-pr', 'github-release']) {
      await executeReleasePlease(manifest, command, false);
    }
    expect(calls).toEqual(['plan-pr', 'plan-release']);
    for (const command of ['release-pr', 'github-release']) {
      await executeReleasePlease(manifest, command, true);
    }
    expect(calls).toEqual(['plan-pr', 'plan-release', 'write-pr', 'write-release']);
    await expect(executeReleasePlease(manifest, 'unknown', false)).rejects.toThrow('Unknown');
    expect(calls).toHaveLength(4);
  });

  // Only SCM acquisition is a fixture. Manifest splitting, excludes, per-product cutoffs,
  // conventional commits, version planning and PR updates are the real pinned implementation.
  async function candidateManifest(
    changes: ReturnType<typeof commit>[],
    wrapped = true,
    legacyAlpha = false,
    taglessPaths: string[] = []
  ) {
    const config = readJson('release-please-config.json');
    if (legacyAlpha) {
      config['prerelease-type'] = 'alpha';
      for (const entry of Object.values(config.packages) as Record<string, unknown>[])
        delete entry['prerelease-type'];
    }
    const versions = { '.': '5.0.0-alpha.8', [squadPath]: '0.1.0-alpha.8' };
    const github = await releasePlease.GitHub.create({
      owner: 'fixture',
      repo: 'fixture',
      defaultBranch: 'main',
    });
    github.getGitHubApi().octokit.hook.before('request', () => {
      throw new Error('Unexpected SCM request in release candidate fixture');
    });
    github.getFileContentsOnBranch = async (file) => {
      const parsedContent = JSON.stringify(
        file === 'release-please-config.json' ? config : versions
      );
      return {
        parsedContent,
        content: Buffer.from(parsedContent).toString('base64'),
        sha: 'fixture',
        mode: '100644',
      };
    };
    github.releaseIterator = async function* () {
      if (!taglessPaths.includes(squadPath))
        yield {
          id: 1,
          url: 'https://example.test/squad',
          name: 'Squad',
          tagName: 'tmt-squad-v0.1.0-alpha.8',
          sha: 'squad-release',
          notes: '',
        };
      if (!taglessPaths.includes('.'))
        yield {
          id: 2,
          url: 'https://example.test/cli',
          name: 'CLI',
          tagName: 'v5.0.0-alpha.8',
          sha: 'cli-release',
          notes: '',
        };
    };
    github.tagIterator = async function* () {};
    github.mergeCommitIterator = async function* (_branch, _options = {}) {
      yield* changes;
    };
    if (wrapped) attributeReleaseConsumption(github, components());
    const manifest = await releasePlease.Manifest.fromManifest(github, 'main');
    return { github, manifest };
  }

  async function candidates(
    changes: ReturnType<typeof commit>[],
    wrapped = true,
    legacyAlpha = false
  ) {
    const { manifest } = await candidateManifest(changes, wrapped, legacyAlpha);
    return manifest.buildPullRequests();
  }

  const history = (files: string[]) => [
    commit('fix', files),
    commit('squad-release', [`${squadPath}/Cargo.toml`], 'chore: release squad'),
    commit('cli-release', ['rust/Cargo.toml'], 'chore: release cli'),
  ];

  it.each([
    { held: [squadPath], retained: ['tmt-cli'] },
    { held: ['.'], retained: ['tmt-squad'] },
    { held: ['.', squadPath], retained: [] },
    { held: [], retained: ['tmt-cli', 'tmt-squad'] },
  ])('holds only $held while regenerating/updating $retained', async ({ held, retained }) => {
    const { github, manifest } = await candidateManifest(
      [
        ...history(['rust/crates/tmt-core/src/lib.rs', `${squadPath}/src/config.rs`]),
        commit(
          'old-fix',
          ['rust/crates/tmt-core/src/lib.rs', `${squadPath}/src/config.rs`],
          'fix: old history'
        ),
      ],
      true,
      false,
      held
    );
    const original = await manifest.buildPullRequests();
    expect(original).toHaveLength(2);
    const existing = original.map((candidate, index) => ({
      number: 100 + index,
      title: 'stale title',
      body: candidate.body.toString(),
      headBranchName: candidate.headRefName,
      baseBranchName: 'main',
      labels: ['autorelease: pending'],
      files: [],
      sha: 'a'.repeat(40),
    }));
    github.pullRequestIterator = async function* (_branch, state) {
      if (state === 'OPEN') yield* existing;
    };
    vi.spyOn(github, 'createPullRequest').mockImplementation(async () => {
      throw new Error('Expected an existing release PR update');
    });
    const mutations = [
      vi.spyOn(github, 'commentOnIssue'),
      vi.spyOn(github, 'addIssueLabels'),
      vi.spyOn(github, 'removeIssueLabels'),
      vi.spyOn(github.getGitHubApi().octokit.pulls, 'update'),
      vi.spyOn(github.getGitHubApi().octokit.issues, 'update'),
      vi.spyOn(github.getGitHubApi().octokit.issues, 'create'),
    ];
    const update = vi
      .spyOn(github, 'updatePullRequest')
      .mockImplementation(async (number, candidate) => ({
        ...existing[0],
        number,
        title: candidate.title.toString(),
        body: candidate.body.toString(),
        headBranchName: candidate.headRefName,
      }));
    holdTaglessDraftCandidates(manifest, held);
    const planned = await manifest.buildPullRequests();
    expect(
      planned.map((candidate) => candidate.headRefName.split('--components--')[1]).sort()
    ).toEqual([...retained].sort());
    for (const candidate of planned) {
      expect(candidate.body.toString()).not.toContain('old history');
      expect(candidate.version?.toString()).toBe(
        candidate.headRefName.endsWith('tmt-cli') ? '5.0.0-alpha.9' : '0.1.0-alpha.9'
      );
      expect(candidate.updates.some(({ path }) => path === '.release-please-manifest.json')).toBe(
        true
      );
    }
    await manifest.createPullRequests();
    // The pinned GitHub port has no closePullRequest; REST pulls.update closes PRs.
    // A filtered existing PR remains open with no comment, label or issue mutation.
    for (const mutation of mutations) expect(mutation).not.toHaveBeenCalled();
    expect(
      update.mock.calls
        .map(([, candidate]) => candidate.headRefName.split('--components--')[1])
        .sort()
    ).toEqual([...retained].sort());
  });

  it('rejects malformed/unknown hold paths and missing candidate path evidence', async () => {
    const { manifest } = await candidateManifest(history(['rust/crates/tmt-core/src/lib.rs']));
    for (const held of [null, '.', [null], ['unknown']])
      expect(() => holdTaglessDraftCandidates(manifest, held)).toThrow('manifest paths');
    const [candidate] = await manifest.buildPullRequests();
    holdTaglessDraftCandidates(manifest, ['.']);
    await expect(
      manifest.plugins[0].run([
        { path: 'unknown', pullRequest: candidate, config: manifest.repositoryConfig['.'] },
      ])
    ).rejects.toThrow('candidate path');
    const { manifest: combined } = await candidateManifest(
      history(['rust/crates/tmt-core/src/lib.rs'])
    );
    Object.defineProperty(combined, 'separatePullRequests', { value: false });
    expect(() => holdTaglessDraftCandidates(combined, ['.'])).toThrow('separate release PR');
  });

  it('proposes only Squad for a TUI-only fix and fails without the attribution step', async () => {
    const changes = history([`${leafPath}/src/binding.rs`]);
    expect(await candidates(changes, false)).toEqual([]);
    const proposed = await candidates(changes);
    expect(proposed).toHaveLength(1);
    expect(proposed[0].title.toString()).toContain('tmt-squad');
    expect(proposed[0].version?.toString()).toBe('0.1.0-alpha.9');
    const updates = proposed[0].updates.map(({ path }) => path);
    expect(updates).toContain('extensions/tmt-squad/rust/tmt-squad/Cargo.toml');
    expect(updates).not.toContain('rust/Cargo.toml');
    expect(changes[0].files).toEqual([`${leafPath}/src/binding.rs`]);
  });

  it('preserves candidate versions when alpha moves from the schema-invalid root into packages', async () => {
    const changes = history([`${leafPath}/src/binding.rs`, 'rust/crates/tmt-core/src/lib.rs']);
    const current = await candidates(changes);
    const legacy = await candidates(changes, true, true);
    // Cover every active release component, not merely whichever candidates happen to appear.
    expect(current).toHaveLength(
      Object.keys(readJson('release-please-config.json').packages).length
    );
    expect(current.map((pr) => pr.version?.toString()).sort()).toEqual([
      '0.1.0-alpha.9',
      '5.0.0-alpha.9',
    ]);
    expect(current.map((pr) => pr.version?.toString())).toEqual(
      legacy.map((pr) => pr.version?.toString())
    );
  });

  it('leaves core-only proposals unchanged and unrelated private leaves unpublished', async () => {
    for (const file of [
      'rust/crates/tmt-core/src/lib.rs',
      'extensions/tmt-remote/rust/tmt-remote/src/main.rs',
      `${leafPath}-other/src/lib.rs`,
    ]) {
      const plain = await candidates(history([file]), false);
      const wrapped = await candidates(history([file]));
      expect(wrapped.map((pr) => pr.title.toString())).toEqual(
        plain.map((pr) => pr.title.toString())
      );
      if (file.startsWith('extensions/tmt-remote/')) expect(wrapped).toEqual([]);
    }
  });

  it('does not replay a TUI fix older than the Squad release even when CLI history extends further', async () => {
    const changes = [
      commit('squad-release', [`${squadPath}/Cargo.toml`], 'chore: release squad'),
      commit('old-fix', [`${leafPath}/src/binding.rs`]),
      commit('cli-release', ['rust/Cargo.toml'], 'chore: release cli'),
    ];
    expect(await candidates(changes)).toEqual([]);
  });

  it('retains both proposals when a commit also changes core and deduplicates Squad attribution', async () => {
    const proposed = await candidates(
      history([
        `${leafPath}/src/binding.rs`,
        `${squadPath}/src/view.rs`,
        'rust/crates/tmt-core/src/lib.rs',
      ])
    );
    expect(proposed).toHaveLength(2);
    expect(proposed.map((pr) => pr.version?.toString()).sort()).toEqual([
      '0.1.0-alpha.9',
      '5.0.0-alpha.9',
    ]);
  });

  async function updateFixture() {
    const [candidate] = await candidates(history(['rust/crates/tmt-core/src/lib.rs']));
    const github = await releasePlease.GitHub.create({
      owner: 'fixture',
      repo: 'fixture',
      defaultBranch: 'main',
    });
    const sha = 'a'.repeat(40);
    const snapshot = {
      state: 'open',
      title: candidate.title.toString(),
      body: candidate.body.toString(),
      mergeable: true as boolean | null,
      head: { ref: candidate.headRefName, sha, repo: { full_name: 'fixture/fixture' } },
      base: { ref: 'main' },
    };
    const existing = {
      number: 17,
      title: snapshot.title,
      body: snapshot.body,
      headBranchName: snapshot.head.ref,
      baseBranchName: 'main',
      labels: ['autorelease: pending'],
      files: [],
      sha,
    };
    const generated = new Map<string, { content: string; mode: string }>();
    const readFile = vi
      .spyOn(github, 'getFileContentsOnBranch')
      .mockImplementation(async (file, ref) => {
        let content: string,
          mode = '100644';
        if (ref === sha) {
          const value = generated.get(file);
          if (!value) throw new releasePlease.Errors.FileNotFoundError(file);
          content = value.content;
          mode = value.mode;
        } else {
          expect(ref).toBe('main');
          if (!existsSync(path.join(root, file)))
            throw new releasePlease.Errors.FileNotFoundError(file);
          content = read(file);
        }
        return {
          content: Buffer.from(content).toString('base64'),
          parsedContent: content,
          mode,
          sha: 'fixture-file',
        };
      });
    for (const [file, value] of await github.buildChangeSet(candidate.updates, 'main')) {
      expect(value.content).not.toBeNull();
      generated.set(file, { content: value.content as string, mode: value.mode });
    }
    expect(generated.has('rust/Cargo.lock')).toBe(true);
    expect(generated.has('.release-please-manifest.json')).toBe(true);
    const getSnapshot = vi.fn(async () => ({ data: snapshot }));
    Object.defineProperty(github.getGitHubApi().octokit.pulls, 'get', { value: getSnapshot });
    vi.spyOn(github, 'getPullRequest').mockResolvedValue(existing);
    const mutation = vi.spyOn(github, 'updatePullRequest').mockResolvedValue(existing);
    preserveUnchangedReleasePullRequests(github, releasePlease.Errors.FileNotFoundError);
    return {
      github,
      candidate,
      snapshot,
      generated,
      mutation,
      readFile,
      existing,
      update: () => github.updatePullRequest(17, candidate, 'main', { fork: false }),
    };
  }

  it('preserves an unchanged release head across repeated main pushes using the pinned file updaters', async () => {
    const fixture = await updateFixture();
    for (let push = 0; push < 3; push += 1)
      expect(await fixture.update()).toEqual(fixture.existing);
    expect(fixture.mutation).not.toHaveBeenCalled();
    expect(fixture.snapshot.head.sha).toBe('a'.repeat(40));
    const comparisons = fixture.readFile.mock.calls.filter(
      ([, ref]) => ref === fixture.snapshot.head.sha
    );
    expect(comparisons.length).toBe(fixture.generated.size * 3);
  });

  it.each(['title', 'notes', 'lock', 'manifest', 'mode', 'missing', 'conflict', 'closed'])(
    'still invokes the original updater for changed release content or recovery: %s',
    async (reason) => {
      const fixture = await updateFixture();
      if (reason === 'title') fixture.snapshot.title += ' stale';
      if (reason === 'notes') fixture.snapshot.body += ' stale';
      if (reason === 'conflict') fixture.snapshot.mergeable = false;
      if (reason === 'closed') fixture.snapshot.state = 'closed';
      if (reason === 'missing') fixture.generated.delete('rust/Cargo.lock');
      const file = reason === 'manifest' ? '.release-please-manifest.json' : 'rust/Cargo.lock';
      const generated = fixture.generated.get(file);
      if (reason === 'lock' || reason === 'manifest') generated!.content += ' changed';
      if (reason === 'mode') generated!.mode = '100755';
      await fixture.update();
      expect(fixture.mutation).toHaveBeenCalledExactlyOnceWith(17, fixture.candidate, 'main', {
        fork: false,
      });
    }
  );

  it('returns the existing PR without rewriting when unchanged mergeability is unknown', async () => {
    const fixture = await updateFixture();
    fixture.snapshot.mergeable = null;
    expect(await fixture.update()).toEqual(fixture.existing);
    expect(fixture.mutation).not.toHaveBeenCalled();
  });

  it('does not mistake a file-read failure for content equality or rewrite permission', async () => {
    const fixture = await updateFixture();
    fixture.readFile.mockImplementation(async () => {
      throw new Error('API unavailable');
    });
    await expect(fixture.update()).rejects.toThrow('API unavailable');
    expect(fixture.mutation).not.toHaveBeenCalled();
  });

  it('refuses a changed branch identity before a mutation', async () => {
    const fixture = await updateFixture();
    fixture.snapshot.head.ref = 'foreign-branch';
    await expect(fixture.update()).rejects.toThrow('no longer matches');
    expect(fixture.mutation).not.toHaveBeenCalled();
  });

  it('propagates the original rewrite failure for a changed release', async () => {
    const fixture = await updateFixture();
    fixture.snapshot.body += ' stale';
    fixture.mutation.mockRejectedValue(new Error('rewrite refused'));
    await expect(fixture.update()).rejects.toThrow('rewrite refused');
  });

  it('fails loudly for absent backfilled commit files', async () => {
    const changes = history([]);
    Reflect.deleteProperty(changes[0], 'files');
    await expect(candidates(changes)).rejects.toThrow('missing valid backfilled files');
  });
});
