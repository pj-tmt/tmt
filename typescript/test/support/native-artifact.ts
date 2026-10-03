import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import path from 'node:path';
import * as tar from 'tar';
import { writeExecutable } from './executable-fixture.mjs';

// The archive's file list has one owner, the artifact policy the release
// verifiers use; fixtures build from it rather than copying it.
const { runtimeFiles, companionFiles } = (await import(
  new URL('../../scripts/native-artifact-policy.mjs', import.meta.url).href
)) as unknown as {
  runtimeFiles: (
    product?: 'cli' | 'office' | 'squad' | 'remote' | 'colab' | 'driver-herdr'
  ) => string[];
  companionFiles: (
    product?: 'cli' | 'office' | 'squad' | 'remote' | 'colab' | 'driver-herdr'
  ) => string[];
};

export type ArtifactFixture = {
  readonly archive: string;
  readonly manifest: string;
  readonly version: string;
  readonly target: string;
};

export type ArtifactSources = {
  readonly root: string;
  /** Override the synthetic installation note, for archive-local fixture configuration. */
  readonly installationNote?: string;
  readonly cli?: {
    readonly executable: string;
    /**
     * Where the build's companions are, by default beside `executable`; `null` builds a
     * published archive from before companions existed, such as CLI 5.0.0-alpha.39.
     */
    readonly companions?: string | null;
  };
};

export function nativeTarget(): string {
  const architecture =
    process.arch === 'arm64' ? 'aarch64' : process.arch === 'x64' ? 'x86_64' : null;
  const platform =
    process.platform === 'darwin'
      ? 'apple-darwin'
      : process.platform === 'linux'
        ? 'unknown-linux-musl'
        : null;
  if (architecture === null || platform === null)
    throw new Error(`Unsupported native test target: ${process.arch}-${process.platform}`);
  return `${architecture}-${platform}`;
}

export async function createArtifact(
  sources: ArtifactSources,
  version: string,
  executableSuffix: Uint8Array = new Uint8Array(),
  product: 'cli' | 'office' | 'squad' | 'remote' | 'colab' | 'driver-herdr' = 'cli',
  companionExecutable = path.resolve('../rust/target/debug/tmt-office'),
  /** An extension's agent-skills tree, by path under `skills/`. */
  skills: Record<string, string> = {},
  archiveName?: string
): Promise<ArtifactFixture> {
  const target = nativeTarget();
  const name = archiveName ?? `${product}-${version}-${target}.tar.gz`;
  const fixtureRoot = path.join(sources.root, 'native archive inputs with spaces', product);
  const tree = path.join(fixtureRoot, 'tree');
  const root = path.join(tree, name.slice(0, -'.tar.gz'.length));
  const archive = path.join(fixtureRoot, name);
  const manifest = path.join(fixtureRoot, 'manifest.json');
  mkdirSync(root, { recursive: true });
  const executableName = product === 'cli' ? 'tmt' : `tmt-${product}`;
  // Extensions are built independently. Never substitute the CLI for a missing one.
  const source =
    product === 'cli'
      ? sources.cli?.executable
      : product === 'office'
        ? companionExecutable
        : path.resolve(`../rust/target/debug/tmt-${product}`);
  if (source === undefined) throw new Error(`Missing ${product} executable for artifact fixture.`);
  const executable = path.join(root, executableName);
  writeExecutable(executable, Buffer.concat([readFileSync(source), executableSuffix]));
  writeFileSync(path.join(root, 'LICENSE'), 'MIT\n');
  writeFileSync(
    path.join(root, 'NATIVE-INSTALL.md'),
    sources.installationNote ?? 'Native local installation fixture.\n'
  );
  writeFileSync(path.join(root, 'THIRD-PARTY-NOTICES.txt'), 'Synthetic test notice fixture.\n');
  // A companion (the CLI's Herdr driver) comes from the same build as the
  // executable, as in a release archive.
  const companions = sources.cli?.companions === null ? [] : companionFiles(product);
  for (const companion of companions) {
    const built = sources.cli?.companions ?? path.dirname(source);
    writeExecutable(path.join(root, companion), readFileSync(path.join(built, companion)), 0o755);
  }
  for (const [file, content] of Object.entries(skills)) {
    mkdirSync(path.dirname(path.join(root, 'skills', file)), { recursive: true });
    writeFileSync(path.join(root, 'skills', file), content);
  }
  await tar.c({ cwd: tree, file: archive, gzip: true }, [path.basename(root)]);
  const checksum = createHash('sha256').update(readFileSync(archive)).digest('hex');
  writeFileSync(
    manifest,
    `${JSON.stringify({
      artifacts: {
        [name]: {
          kind: 'executable-zip',
          name,
          target_triples: [target],
          checksums: { sha256: checksum },
          // cargo-dist declares an included directory as one asset.
          assets: [
            ...runtimeFiles(product),
            ...companions,
            ...(Object.keys(skills).length > 0 ? ['skills'] : []),
          ].map((file) => ({ path: file })),
        },
      },
      releases: [
        {
          app_name: product === 'cli' ? 'tmt-cli' : `tmt-${product}`,
          app_version: version,
          artifacts: [name],
        },
      ],
    })}\n`
  );
  return { archive, manifest, version, target };
}
