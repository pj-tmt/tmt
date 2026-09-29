import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import * as tar from 'tar';

const REQUIRED_FILES = ['tmt', 'LICENSE', 'NATIVE-INSTALL.md', 'THIRD-PARTY-NOTICES.txt'];

export type ArtifactFixture = {
  readonly archive: string;
  readonly manifest: string;
  readonly version: string;
  readonly target: string;
};

export type ArtifactSources = {
  readonly root: string;
  readonly cli?: { readonly executable: string };
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
  product: 'cli' | 'office' | 'squad' = 'cli',
  companionExecutable = path.resolve('../rust/target/debug/tmt-office')
): Promise<ArtifactFixture> {
  const target = nativeTarget();
  const name = `${product}-${version}-${target}.tar.gz`;
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
        : path.resolve('../rust/target/debug/tmt-squad');
  if (source === undefined) throw new Error(`Missing ${product} executable for artifact fixture.`);
  copyFileSync(source, path.join(root, executableName));
  const executable = path.join(root, executableName);
  chmodSync(executable, 0o755);
  if (executableSuffix.byteLength > 0)
    writeFileSync(executable, Buffer.concat([readFileSync(executable), executableSuffix]));
  writeFileSync(path.join(root, 'LICENSE'), 'MIT\n');
  writeFileSync(path.join(root, 'NATIVE-INSTALL.md'), 'Native local installation fixture.\n');
  writeFileSync(path.join(root, 'THIRD-PARTY-NOTICES.txt'), 'Synthetic test notice fixture.\n');
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
          assets: REQUIRED_FILES.map((file) => ({ path: file === 'tmt' ? executableName : file })),
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
