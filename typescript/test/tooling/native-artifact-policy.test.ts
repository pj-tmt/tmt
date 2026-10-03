import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { pipeline } from 'node:stream/promises';
import { Readable } from 'node:stream';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createGzip } from 'node:zlib';
import { describe, expect, it, vi } from 'vite-plus/test';
import * as tar from 'tar';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const { selectNativeArtifact, withNativeArtifact } = (await import(
  pathToFileURL(path.join(repositoryRoot, 'scripts', 'native-artifact-policy.mjs')).href
)) as unknown as {
  selectNativeArtifact: (
    manifestFile: string,
    archiveFile: string,
    target: string,
    product?: 'cli' | 'office' | 'squad' | 'remote' | 'colab' | 'driver-herdr',
    options?: { readonly release?: boolean }
  ) => NativeArtifact;
  withNativeArtifact: <T>(
    archiveFile: string,
    metadata: NativeArtifact,
    inspect: (root: string) => Promise<T> | T
  ) => Promise<T>;
};

const REQUIRED_FILES = ['tmt', 'LICENSE', 'NATIVE-INSTALL.md', 'THIRD-PARTY-NOTICES.txt'];
/** Product::companions(): executables a product's archive carries beside its own. */
const COMPANIONS: Record<
  'cli' | 'office' | 'squad' | 'remote' | 'colab' | 'driver-herdr',
  readonly string[]
> = {
  cli: ['tmt-driver-herdr'],
  office: [],
  squad: [],
  remote: [],
  colab: [],
  'driver-herdr': [],
};
const TARGET = 'aarch64-apple-darwin';
const VERSION = '5.0.0-alpha.1';

interface NativeArtifact {
  readonly name: string;
  readonly version: string;
  readonly target: string;
  readonly sha256: string;
}

interface ArchiveOptions {
  readonly product?: 'cli' | 'office' | 'squad' | 'remote' | 'colab' | 'driver-herdr';
  /** Files under `skills/`, by path relative to it. */
  readonly skills?: Record<string, string>;
  /** Whether the manifest declares the `skills` asset (default: when Squad). */
  readonly declareSkills?: boolean;
  readonly skillLink?: boolean;
  readonly duplicate?: string;
  readonly executable?: boolean;
  /** Whether the companion executables keep their execute bit (default true). */
  readonly companionExecutable?: boolean;
  /** Whether the manifest lists the companion executables (default true). */
  readonly declareCompanions?: boolean;
  readonly extra?: string;
  readonly link?: 'hard' | 'symlink';
  readonly omit?: string;
  readonly specialPermissions?: boolean;
  readonly traversal?: boolean;
}

interface ArchiveFixture {
  readonly archiveFile: string;
  readonly manifestFile: string;
  readonly manifest: Record<string, unknown>;
  readonly metadata: NativeArtifact;
}

function archiveName(target = TARGET): string {
  return `tmux-team-${VERSION}-${target}.tar.gz`;
}

async function createArchiveFixture(
  sandbox: Sandbox,
  options: ArchiveOptions = {}
): Promise<ArchiveFixture> {
  const name = archiveName();
  const rootName = name.slice(0, -'.tar.gz'.length);
  const fixtureRoot = fs.mkdtempSync(path.join(sandbox.root, 'archive-fixture-'));
  const tree = path.join(fixtureRoot, 'archive tree');
  const root = path.join(tree, rootName);
  const archiveFile = path.join(fixtureRoot, name);
  const manifestFile = path.join(fixtureRoot, 'manifest.json');
  fs.mkdirSync(root, { recursive: true });

  const executable = {
    cli: 'tmt',
    office: 'tmt-office',
    squad: 'tmt-squad',
    remote: 'tmt-remote',
    colab: 'tmt-colab',
    'driver-herdr': 'tmt-driver-herdr',
  }[options.product ?? 'cli'];
  const companions = COMPANIONS[options.product ?? 'cli'];
  const files = [executable, ...REQUIRED_FILES.slice(1), ...companions];
  for (const file of files) {
    if (file === options.omit) continue;
    const target = path.join(root, file);
    const runs =
      file === executable || (companions.includes(file) && options.companionExecutable !== false);
    fs.writeFileSync(target, `${file} fixture\n`, { mode: runs ? 0o755 : 0o644 });
  }
  for (const [file, content] of Object.entries(options.skills ?? {})) {
    fs.mkdirSync(path.dirname(path.join(root, 'skills', file)), { recursive: true });
    fs.writeFileSync(path.join(root, 'skills', file), content);
  }
  if (options.skillLink) {
    fs.mkdirSync(path.join(root, 'skills', 'tmt-squad'), { recursive: true });
    fs.symlinkSync('../../../outside', path.join(root, 'skills', 'tmt-squad', 'link.md'));
  }
  if (options.executable === false) fs.chmodSync(path.join(root, executable), 0o644);
  if (options.specialPermissions) fs.chmodSync(path.join(root, 'tmt'), 0o4755);
  if (options.extra !== undefined) {
    fs.writeFileSync(path.join(root, options.extra), 'unexpected test fixture\n');
  }
  if (options.link === 'symlink') {
    fs.unlinkSync(path.join(root, 'LICENSE'));
    fs.symlinkSync('../../outside', path.join(root, 'LICENSE'));
  } else if (options.link === 'hard') {
    fs.unlinkSync(path.join(root, 'LICENSE'));
    fs.linkSync(path.join(root, 'tmt'), path.join(root, 'LICENSE'));
  }

  const entries = [rootName];
  const tarOptions = {
    cwd: tree,
    file: archiveFile,
    gzip: true,
    portable: options.specialPermissions !== true,
    // tar's asynchronous hard-link queue can strand a pending entry when
    // filesystem callbacks complete out of order. These fixtures are tiny.
    sync: options.link === 'hard',
  };
  if (options.traversal) {
    fs.writeFileSync(path.join(fixtureRoot, 'escape'), 'traversal fixture\n');
    entries.push('../escape');
    Object.assign(tarOptions, { preservePaths: true });
  }
  if (options.specialPermissions) {
    Object.assign(tarOptions, {
      onWriteEntry(entry: { path: string; stat: { mode: number } }) {
        if (entry.path === `${rootName}/tmt`) entry.stat.mode = 0o4755;
      },
    });
  }
  if (options.duplicate !== undefined) entries.push(path.join(rootName, options.duplicate));
  await tar.c(tarOptions, entries);
  const sha256 = createHash('sha256').update(fs.readFileSync(archiveFile)).digest('hex');
  const manifest = {
    artifacts: {
      [name]: {
        kind: 'executable-zip',
        name,
        target_triples: [TARGET],
        checksums: { sha256 },
        assets: [
          ...files.filter(
            (file) => options.declareCompanions !== false || !companions.includes(file)
          ),
          ...((options.declareSkills ?? options.product === 'squad') ? ['skills'] : []),
        ].map((file) => ({ path: file })),
      },
    },
    releases: [
      { app_name: `tmt-${options.product ?? 'cli'}`, app_version: VERSION, artifacts: [name] },
    ],
  };
  fs.writeFileSync(manifestFile, `${JSON.stringify(manifest)}\n`);
  return {
    archiveFile,
    manifestFile,
    manifest,
    metadata: selectNativeArtifact(manifestFile, archiveFile, TARGET, options.product),
  };
}

function withArtifactMetadata(sha256: string): NativeArtifact {
  return {
    name: archiveName(),
    version: VERSION,
    target: TARGET,
    sha256,
  };
}

describe('native artifact policy', () => {
  it.each(['remote', 'colab'] as const)(
    'verifies %s without skills or companions and rejects another product or missing executable',
    async (product) => {
      await withSandbox(async (sandbox) => {
        const artifact = await createArchiveFixture(sandbox, { product });
        await withNativeArtifact(artifact.archiveFile, artifact.metadata, (root) => {
          expect(fs.readdirSync(root).sort()).toEqual([
            'LICENSE',
            'NATIVE-INSTALL.md',
            'THIRD-PARTY-NOTICES.txt',
            `tmt-${product}`,
          ]);
        });
        for (const other of ['cli', 'office', 'squad', 'remote', 'colab'] as const) {
          if (other === product) continue;
          expect(() =>
            selectNativeArtifact(artifact.manifestFile, artifact.archiveFile, TARGET, other)
          ).toThrow('Archive must belong to the TMT release');
        }
        const missing = await createArchiveFixture(sandbox, {
          product,
          omit: `tmt-${product}`,
        });
        await expect(
          withNativeArtifact(missing.archiveFile, missing.metadata, () => undefined)
        ).rejects.toThrow('Missing native archive entry:');
        await expect(
          createArchiveFixture(sandbox, { product, declareSkills: true })
        ).rejects.toThrow('Manifest must describe exactly the native runtime files');
      });
    }
  );
  it('verifies a Squad archive with its declared skills tree and rejects any other tree', async () => {
    await withSandbox(async (sandbox) => {
      const skills = {
        'tmt-squad/SKILL.md': 'lead skill\n',
        'tmt-squad/references/usage.md': 'usage\n',
      };
      const squad = await createArchiveFixture(sandbox, { product: 'squad', skills });
      await withNativeArtifact(squad.archiveFile, squad.metadata, (root) => {
        expect(fs.readdirSync(root).sort()).toEqual([
          'LICENSE',
          'NATIVE-INSTALL.md',
          'THIRD-PARTY-NOTICES.txt',
          'skills',
          'tmt-squad',
        ]);
        expect(
          fs.readFileSync(path.join(root, 'skills/tmt-squad/references/usage.md'), 'utf8')
        ).toBe('usage\n');
      });
      // Squad must declare the tree; nothing else may carry one.
      await expect(
        createArchiveFixture(sandbox, { product: 'squad', skills, declareSkills: false })
      ).rejects.toThrow('Manifest must describe exactly the native runtime files');
      const cli = await createArchiveFixture(sandbox, { skills });
      await expect(
        withNativeArtifact(cli.archiveFile, cli.metadata, () => undefined)
      ).rejects.toThrow('Unexpected native archive entry');
      const empty = await createArchiveFixture(sandbox, { product: 'squad' });
      await expect(
        withNativeArtifact(empty.archiveFile, empty.metadata, () => undefined)
      ).rejects.toThrow('Native archive declares skills but carries none');
      const linked = await createArchiveFixture(sandbox, {
        product: 'squad',
        skills,
        skillLink: true,
      });
      await expect(
        withNativeArtifact(linked.archiveFile, linked.metadata, () => undefined)
      ).rejects.toThrow('Native archive entry must be regular');
      await expect(createArchiveFixture(sandbox, { skills, declareSkills: true })).rejects.toThrow(
        'Manifest must describe exactly the native runtime files'
      );
    });
  });

  it('verifies Office through the shared allowlist without accepting CLI ownership or a non-executable payload', async () => {
    await withSandbox(async (sandbox) => {
      const office = await createArchiveFixture(sandbox, { product: 'office' });
      await withNativeArtifact(office.archiveFile, office.metadata, (root) => {
        expect(fs.readdirSync(root).sort()).toEqual([
          'LICENSE',
          'NATIVE-INSTALL.md',
          'THIRD-PARTY-NOTICES.txt',
          'tmt-office',
        ]);
        expect(fs.readFileSync(path.join(root, 'tmt-office'), 'utf8')).toBe('tmt-office fixture\n');
      });
      expect(() => selectNativeArtifact(office.manifestFile, office.archiveFile, TARGET)).toThrow(
        'Archive must belong to the TMT release'
      );
      const cli = await createArchiveFixture(sandbox);
      expect(() =>
        selectNativeArtifact(cli.manifestFile, cli.archiveFile, TARGET, 'office')
      ).toThrow('Archive must belong to the TMT release');
      const invalid = await createArchiveFixture(sandbox, { product: 'office', executable: false });
      await expect(
        withNativeArtifact(invalid.archiveFile, invalid.metadata, () => undefined)
      ).rejects.toThrow('Native executable lacks execute permission');
    });
  });
  it.each([
    ['notices', 'stale notices', 'Native archive notices differ from the generated inventory'],
    ['license', 'stale license', 'Native archive license differs from the selected source'],
    ['notices', 'Copyright (c) <year>', 'Dependency notices contain placeholder attribution'],
  ])('fails the verifier process for invalid %s evidence', async (changed, content, diagnostic) => {
    await withSandbox(async (sandbox) => {
      const fixture = await createArchiveFixture(sandbox);
      const architecture = process.arch === 'arm64' ? 'aarch64' : 'x86_64';
      const platform = process.platform === 'darwin' ? 'apple-darwin' : 'unknown-linux-musl';
      const target = `${architecture}-${platform}`;
      const artifact = (fixture.manifest.artifacts as Record<string, Record<string, unknown>>)[
        archiveName()
      ];
      artifact.target_triples = [target];
      fs.writeFileSync(fixture.manifestFile, JSON.stringify(fixture.manifest));
      const notices = path.join(sandbox.root, 'selected-notices.txt');
      const license = path.join(sandbox.root, 'selected-license.txt');
      fs.writeFileSync(
        notices,
        changed === 'notices' ? content : 'THIRD-PARTY-NOTICES.txt fixture\n'
      );
      fs.writeFileSync(license, changed === 'license' ? content : 'LICENSE fixture\n');
      const result = await runCli(
        {
          ...sandbox,
          cli: {
            executable: process.execPath,
            args: [path.join(repositoryRoot, 'scripts/verify-native-artifact.mjs')],
          },
        },
        [
          '--manifest',
          fixture.manifestFile,
          '--archive',
          fixture.archiveFile,
          '--target',
          target,
          '--skill',
          license,
          '--notices',
          notices,
          '--license',
          license,
        ]
      );
      expect(result.status).toBe(1);
      expect(result.stdout).toBe('');
      expect(result.stderr).toContain(diagnostic);
    });
  });

  it('rejects a manifest target mismatch and a missing checksum', async () => {
    await withSandbox(async (sandbox) => {
      const targetFixture = await createArchiveFixture(sandbox);
      const mismatched = structuredClone(targetFixture.manifest) as Record<string, unknown>;
      const artifact = (mismatched.artifacts as Record<string, Record<string, unknown>>)[
        archiveName()
      ];
      artifact.target_triples = ['x86_64-unknown-linux-musl'];
      fs.writeFileSync(targetFixture.manifestFile, JSON.stringify(mismatched));
      expect(() =>
        selectNativeArtifact(targetFixture.manifestFile, targetFixture.archiveFile, TARGET)
      ).toThrow('Manifest target mismatch');

      const checksumFixture = await createArchiveFixture(sandbox);
      const missingChecksum = structuredClone(checksumFixture.manifest) as Record<string, unknown>;
      const missingArtifact = (
        missingChecksum.artifacts as Record<string, Record<string, unknown>>
      )[archiveName()];
      missingArtifact.checksums = {};
      fs.writeFileSync(checksumFixture.manifestFile, JSON.stringify(missingChecksum));
      expect(() =>
        selectNativeArtifact(checksumFixture.manifestFile, checksumFixture.archiveFile, TARGET)
      ).toThrow('Manifest requires SHA-256');
    });
  });

  it('rejects malformed asset inventories and release membership', async () => {
    await withSandbox(async (sandbox) => {
      const assetFixture = await createArchiveFixture(sandbox);
      const malformedAssets = structuredClone(assetFixture.manifest) as Record<string, unknown>;
      const assetArtifact = (malformedAssets.artifacts as Record<string, Record<string, unknown>>)[
        archiveName()
      ];
      assetArtifact.assets = [{ path: 'tmt' }];
      fs.writeFileSync(assetFixture.manifestFile, JSON.stringify(malformedAssets));
      expect(() =>
        selectNativeArtifact(assetFixture.manifestFile, assetFixture.archiveFile, TARGET)
      ).toThrow('Manifest must describe exactly the native runtime files');

      const releaseFixture = await createArchiveFixture(sandbox);
      const missingRelease = structuredClone(releaseFixture.manifest) as Record<string, unknown>;
      missingRelease.releases = [];
      fs.writeFileSync(releaseFixture.manifestFile, JSON.stringify(missingRelease));
      expect(() =>
        selectNativeArtifact(releaseFixture.manifestFile, releaseFixture.archiveFile, TARGET)
      ).toThrow('Archive must belong to exactly one release');

      const wrongOwner = structuredClone(releaseFixture.manifest);
      (wrongOwner.releases as Array<Record<string, unknown>>)[0].app_name = 'unrelated-app';
      fs.writeFileSync(releaseFixture.manifestFile, JSON.stringify(wrongOwner));
      expect(() =>
        selectNativeArtifact(releaseFixture.manifestFile, releaseFixture.archiveFile, TARGET)
      ).toThrow('Archive must belong to the TMT release');
    });
  });

  it('accepts four regular files, returns the inspector value, and cleans staging', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createArchiveFixture(sandbox);
      const sentinel = path.join(sandbox.root, 'unrelated sentinel.txt');
      fs.writeFileSync(sentinel, 'preserve me\n');
      let extracted: string | undefined;

      const result = await withNativeArtifact(fixture.archiveFile, fixture.metadata, (root) => {
        extracted = root;
        expect(fs.readdirSync(root).sort()).toEqual([...REQUIRED_FILES, ...COMPANIONS.cli].sort());
        expect(fs.readFileSync(path.join(root, 'LICENSE'), 'utf8')).toBe('LICENSE fixture\n');
        expect(fs.statSync(path.join(root, 'tmt')).mode & 0o111).not.toBe(0);
        expect(fs.readFileSync(sentinel, 'utf8')).toBe('preserve me\n');
        return 'inspected';
      });

      expect(result).toBe('inspected');
      expect(extracted).toBeDefined();
      expect(fs.existsSync(extracted as string)).toBe(false);
      expect(fs.readFileSync(sentinel, 'utf8')).toBe('preserve me\n');
    });
  });

  it('rejects an archive checksum mismatch before invoking the callback', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createArchiveFixture(sandbox);
      const original = fs.readFileSync(fixture.archiveFile);
      let called = false;
      const metadata = { ...fixture.metadata, sha256: '0'.repeat(64) };

      await expect(
        withNativeArtifact(fixture.archiveFile, metadata, () => {
          called = true;
        })
      ).rejects.toThrow('Native archive checksum mismatch');
      expect(called).toBe(false);
      expect(fs.readFileSync(fixture.archiveFile)).toEqual(original);
    });
  });

  it('rejects an invalid metadata archive name before reading the archive', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createArchiveFixture(sandbox);
      const original = fs.readFileSync(fixture.archiveFile);
      let called = false;

      await expect(
        withNativeArtifact(fixture.archiveFile, { ...fixture.metadata, name: '...tar.gz' }, () => {
          called = true;
        })
      ).rejects.toThrow('Invalid native archive name');
      expect(called).toBe(false);
      expect(fs.readFileSync(fixture.archiveFile)).toEqual(original);
    });
  });

  it('rejects oversized compressed input before allocating its contents', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createArchiveFixture(sandbox);
      const descriptor = fs.openSync(fixture.archiveFile, 'r+');
      try {
        fs.ftruncateSync(descriptor, 64 * 1024 * 1024 + 1);
      } finally {
        fs.closeSync(descriptor);
      }
      let called = false;

      await expect(
        withNativeArtifact(fixture.archiveFile, fixture.metadata, () => {
          called = true;
        })
      ).rejects.toThrow('Artifact input must be a bounded regular file');
      expect(called).toBe(false);
    });
  });

  it('rejects a truncated gzip after recomputing its checksum', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createArchiveFixture(sandbox);
      const compressed = fs.readFileSync(fixture.archiveFile);
      const truncated = compressed.subarray(0, compressed.length - 8);
      fs.writeFileSync(fixture.archiveFile, truncated);
      const metadata = withArtifactMetadata(createHash('sha256').update(truncated).digest('hex'));

      await expect(
        withNativeArtifact(fixture.archiveFile, metadata, () => undefined)
      ).rejects.toThrow(/unexpected end/i);
    });
  });

  it('rejects missing notices and a non-executable runtime', async () => {
    await withSandbox(async (sandbox) => {
      const missingNotice = await createArchiveFixture(sandbox, {
        omit: 'THIRD-PARTY-NOTICES.txt',
      });
      await expect(
        withNativeArtifact(missingNotice.archiveFile, missingNotice.metadata, () => undefined)
      ).rejects.toThrow(
        'Missing native archive entry: tmux-team-5.0.0-alpha.1-aarch64-apple-darwin/THIRD-PARTY-NOTICES.txt'
      );

      const missingExecutable = await createArchiveFixture(sandbox, { omit: 'tmt' });
      await expect(
        withNativeArtifact(
          missingExecutable.archiveFile,
          missingExecutable.metadata,
          () => undefined
        )
      ).rejects.toThrow(
        'Missing native archive entry: tmux-team-5.0.0-alpha.1-aarch64-apple-darwin/tmt'
      );

      const nonExecutable = await createArchiveFixture(sandbox, { executable: false });
      await expect(
        withNativeArtifact(nonExecutable.archiveFile, nonExecutable.metadata, () => undefined)
      ).rejects.toThrow('Native executable lacks execute permission');
    });
  });

  it('reads companions from the archive manifest and requires them all of a release', async () => {
    await withSandbox(async (sandbox) => {
      const release = (fixture: ArchiveFixture) =>
        selectNativeArtifact(fixture.manifestFile, fixture.archiveFile, TARGET, 'cli', {
          release: true,
        });
      const cli = await createArchiveFixture(sandbox);
      expect(release(cli)).toEqual(cli.metadata);
      await withNativeArtifact(cli.archiveFile, cli.metadata, (root) => {
        expect(fs.readdirSync(root)).toContain('tmt-driver-herdr');
        expect(fs.statSync(path.join(root, 'tmt-driver-herdr')).mode & 0o111).not.toBe(0);
      });

      // A published CLI from before the companion (5.0.0-alpha.39 and older) reads as it was
      // released, but is never an archive under release.
      const published = await createArchiveFixture(sandbox, {
        declareCompanions: false,
        omit: 'tmt-driver-herdr',
      });
      await withNativeArtifact(published.archiveFile, published.metadata, (root) => {
        expect(fs.readdirSync(root)).not.toContain('tmt-driver-herdr');
      });
      expect(() => release(published)).toThrow(
        'A release archive must carry every companion executable'
      );

      // A declared companion must be archived and executable; an undeclared one is foreign.
      const missing = await createArchiveFixture(sandbox, { omit: 'tmt-driver-herdr' });
      await expect(
        withNativeArtifact(missing.archiveFile, missing.metadata, () => undefined)
      ).rejects.toThrow(
        'Missing native archive entry: tmux-team-5.0.0-alpha.1-aarch64-apple-darwin/tmt-driver-herdr'
      );
      const nonExecutable = await createArchiveFixture(sandbox, { companionExecutable: false });
      await expect(
        withNativeArtifact(nonExecutable.archiveFile, nonExecutable.metadata, () => undefined)
      ).rejects.toThrow('Native executable lacks execute permission');
      const undeclared = await createArchiveFixture(sandbox, { declareCompanions: false });
      await expect(
        withNativeArtifact(undeclared.archiveFile, undeclared.metadata, () => undefined)
      ).rejects.toThrow('Unexpected native archive entry');

      const squad = await createArchiveFixture(sandbox, {
        product: 'squad',
        skills: { 'tmt-squad/SKILL.md': 'lead skill\n' },
        extra: 'tmt-driver-herdr',
      });
      await expect(
        withNativeArtifact(squad.archiveFile, squad.metadata, () => undefined)
      ).rejects.toThrow('Unexpected native archive entry');
    });
  });

  it('rejects special permission bits on an otherwise executable runtime', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createArchiveFixture(sandbox, { specialPermissions: true });

      await expect(
        withNativeArtifact(fixture.archiveFile, fixture.metadata, () => undefined)
      ).rejects.toThrow('Native archive must not set special permission bits');
    });
  });

  it('rejects a FIFO input through the bounded process helper', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createArchiveFixture(sandbox);
      const fifo = path.join(sandbox.root, 'native archive fifo');
      const mkfifoSandbox = { ...sandbox, cli: { executable: '/usr/bin/mkfifo', args: [] } };
      const created = await runCli(mkfifoSandbox, [fifo]);
      expect(created.status).toBe(0);
      expect(created.signal).toBeNull();
      expect(created.stderr).toBe('');
      expect(fs.statSync(fifo).isFIFO()).toBe(true);
      // Put the subject, not just FIFO creation, behind the process deadline:
      // regressing to a blocking open must fail rather than hang the test runner.
      const moduleUrl = pathToFileURL(
        path.join(repositoryRoot, 'scripts/native-artifact-policy.mjs')
      ).href;
      const probe = `import { withNativeArtifact } from ${JSON.stringify(moduleUrl)};
await withNativeArtifact(process.argv[1], JSON.parse(process.argv[2]), () => console.log('unexpected extraction'));`;
      const result = await runCli(
        {
          ...sandbox,
          cli: { executable: process.execPath, args: ['--input-type=module', '--eval', probe] },
        },
        [fifo, JSON.stringify(fixture.metadata)]
      );
      expect(result.status).toBe(1);
      expect(result.stdout).toBe('');
      expect(result.stderr).toContain('Artifact input must be a bounded regular file');
    });
  });

  it(
    'rejects expanded gzip input over the bound without allocating a giant test buffer',
    { timeout: 5_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const fixture = await createArchiveFixture(sandbox);
        const chunk = Buffer.alloc(256 * 1024, 0x61);
        const source = Readable.from(
          (async function* repeatedChunks() {
            for (let index = 0; index < 513; index += 1) yield chunk;
          })()
        );
        await pipeline(source, createGzip(), fs.createWriteStream(fixture.archiveFile));
        const compressed = fs.readFileSync(fixture.archiveFile);
        const metadata = withArtifactMetadata(
          createHash('sha256').update(compressed).digest('hex')
        );

        await expect(
          withNativeArtifact(fixture.archiveFile, metadata, () => undefined)
        ).rejects.toThrow(/larger than 134217728 bytes/i);
      });
    }
  );

  it('rejects unexpected test files and duplicate entries', async () => {
    await withSandbox(async (sandbox) => {
      const unexpected = await createArchiveFixture(sandbox, { extra: 'native.test.ts' });
      await expect(
        withNativeArtifact(unexpected.archiveFile, unexpected.metadata, () => undefined)
      ).rejects.toThrow('Unexpected native archive entry');

      const duplicate = await createArchiveFixture(sandbox, { duplicate: 'LICENSE' });
      await expect(
        withNativeArtifact(duplicate.archiveFile, duplicate.metadata, () => undefined)
      ).rejects.toThrow('Duplicate native archive entry');
    });
  });

  it('rejects a regular path traversal entry before extraction', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createArchiveFixture(sandbox, { traversal: true });
      await expect(
        withNativeArtifact(fixture.archiveFile, fixture.metadata, () => undefined)
      ).rejects.toThrow('Unexpected native archive entry: ../escape');
    });
  });

  it.each(['symlink', 'hard'] as const)(
    'rejects %s entries, including traversal links',
    async (link) => {
      await withSandbox(async (sandbox) => {
        const fixture = await createArchiveFixture(sandbox, { link });
        await expect(
          withNativeArtifact(fixture.archiveFile, fixture.metadata, () => undefined)
        ).rejects.toThrow('Native archive entry must be regular');
      });
    }
  );

  it('constructs a real hard-link fixture without asynchronous filesystem scheduling', async () => {
    await withSandbox(async (sandbox) => {
      const lstat = vi.spyOn(fs, 'lstat').mockImplementation(() => {
        throw new Error('Hard-link fixture entered the asynchronous tar queue');
      });
      let fixture: ArchiveFixture;
      try {
        fixture = await createArchiveFixture(sandbox, { link: 'hard' });
      } finally {
        lstat.mockRestore();
      }
      const entries: Array<{
        path: string;
        type: string;
        linkpath: string | undefined;
        size: number;
      }> = [];
      tar.t({
        file: fixture.archiveFile,
        sync: true,
        strict: true,
        onReadEntry(entry) {
          entries.push({
            path: entry.path,
            type: entry.type,
            linkpath: entry.linkpath,
            size: entry.size,
          });
        },
      });
      const rootName = archiveName().slice(0, -'.tar.gz'.length);
      expect(entries.filter((entry) => entry.type === 'Link')).toEqual([
        { path: `${rootName}/tmt`, type: 'Link', linkpath: `${rootName}/LICENSE`, size: 0 },
      ]);
      expect(entries.filter((entry) => entry.type === 'File').map((entry) => entry.path)).toEqual(
        ['LICENSE', 'NATIVE-INSTALL.md', 'THIRD-PARTY-NOTICES.txt', ...COMPANIONS.cli].map(
          (file) => `${rootName}/${file}`
        )
      );
      let called = false;
      await expect(
        withNativeArtifact(fixture.archiveFile, fixture.metadata, () => {
          called = true;
        })
      ).rejects.toThrow(`Native archive entry must be regular: ${rootName}/tmt`);
      expect(called).toBe(false);
    });
  });

  it('cleans staging after callback failure without removing an unrelated sentinel', async () => {
    await withSandbox(async (sandbox) => {
      const fixture = await createArchiveFixture(sandbox);
      const sentinel = path.join(sandbox.root, 'unrelated sentinel.txt');
      fs.writeFileSync(sentinel, 'preserve me\n');
      let extracted: string | undefined;

      await expect(
        withNativeArtifact(fixture.archiveFile, fixture.metadata, (root) => {
          extracted = root;
          throw new Error('inspection failed');
        })
      ).rejects.toThrow('inspection failed');
      expect(extracted).toBeDefined();
      expect(fs.existsSync(extracted as string)).toBe(false);
      expect(fs.readFileSync(sentinel, 'utf8')).toBe('preserve me\n');
    });
  });
});

describe('standalone Herdr archive boundary', () => {
  it('accepts its own manifest and refuses CLI ownership or undeclared companion bytes', async () => {
    await withSandbox(async (sandbox) => {
      const standalone = await createArchiveFixture(sandbox, { product: 'driver-herdr' });
      await expect(
        withNativeArtifact(standalone.archiveFile, standalone.metadata, () => 'driver')
      ).resolves.toBe('driver');
      expect(() =>
        selectNativeArtifact(standalone.manifestFile, standalone.archiveFile, TARGET, 'cli')
      ).toThrow('TMT release');
      const extra = await createArchiveFixture(sandbox, { product: 'driver-herdr', extra: 'tmt' });
      await expect(withNativeArtifact(extra.archiveFile, extra.metadata, () => {})).rejects.toThrow(
        'Unexpected native archive entry'
      );
    });
  });
});
