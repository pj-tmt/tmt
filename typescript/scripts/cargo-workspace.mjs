// Cargo owns package identity and dependency kinds; callers own checkout/ref acquisition.
import { dirname, relative, resolve } from 'node:path';
import { runPackedCommand } from './packed-command.mjs';

/** Read an exported repository root, without Git or network access. */
export function readCargoWorkspace(root, { runner = runPackedCommand } = {}) {
  root = resolve(root);
  const metadata = JSON.parse(
    runner(
      'cargo',
      [
        'metadata',
        '--format-version',
        '1',
        '--offline',
        '--locked',
        '--manifest-path',
        resolve(root, 'rust/Cargo.toml'),
      ],
      { cwd: resolve(root, 'rust'), env: process.env, timeoutMs: 60_000 }
    )
  );
  const members = new Set(metadata.workspace_members);
  const crates = metadata.packages.filter((p) => members.has(p.id));
  if (!crates.length || crates.length !== members.size)
    throw new Error('Incomplete Cargo workspace.');
  const byPath = new Map(crates.map((p) => [dirname(p.manifest_path), p.name]));
  const packages = crates.map((crate) => {
    const manifest = relative(root, crate.manifest_path);
    if (manifest.startsWith('../') || manifest === '..')
      throw new Error('Cargo member outside checkout.');
    const dependencies = { normal: [], build: [], dev: [] };
    for (const dependency of crate.dependencies) {
      if (!dependency.path) continue;
      const name = byPath.get(resolve(dependency.path));
      if (!name) continue;
      const kind = dependency.kind ?? 'normal';
      if (!(kind in dependencies)) throw new Error(`Unknown Cargo dependency kind: ${kind}`);
      if (!dependencies[kind].includes(name)) dependencies[kind].push(name);
    }
    return {
      name: crate.name,
      version: crate.version,
      manifestPath: crate.manifest_path,
      manifest,
      dir: dirname(manifest),
      binTargets: crate.targets
        .filter((target) => target.kind.includes('bin'))
        .map((target) => target.name),
      distMetadata: crate.metadata?.dist ?? {},
      dependencies,
    };
  });
  const byName = new Map(packages.map((p) => [p.name, p]));
  if (byName.size !== packages.length) throw new Error('Ambiguous Cargo workspace package names.');
  return {
    packages,
    closure(name, kinds) {
      if (!byName.has(name)) throw new Error(`Unknown Cargo workspace package: ${name}`);
      if (!kinds.length || kinds.some((kind) => !['normal', 'build', 'dev'].includes(kind)))
        throw new Error('Invalid Cargo closure dependency kinds.');
      const seen = new Set();
      const pending = [name];
      while (pending.length) {
        const current = pending.pop();
        if (seen.has(current)) continue;
        seen.add(current);
        for (const kind of kinds) pending.push(...byName.get(current).dependencies[kind]);
      }
      return [...seen].map((entry) => byName.get(entry));
    },
  };
}
