// Git owns the exact source ref; callers retain Cargo and component-map ownership.
import { lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readCargoWorkspace } from './cargo-workspace.mjs';
import { parseComponentMap } from './ci-scope.mjs';
import { runPackedCommand } from './packed-command.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));

/** Read tracked sources at an immutable cut without executing its code or creating a worktree. */
export function withReleaseSourceAtRef(
  sha,
  read,
  { root = ROOT, execute = runPackedCommand } = {}
) {
  if (!/^[a-f0-9]{40}$/.test(sha ?? ''))
    throw new Error('Release source needs an exact commit SHA.');
  const directory = mkdtempSync(join(tmpdir(), 'tmt-release-source-'));
  const source = join(directory, 'source');
  const archive = join(directory, 'source.tar');
  const run = (command, args, cwd = root) =>
    execute(command, args, {
      cwd,
      env: process.env,
      timeoutMs: 30_000,
    });
  try {
    run('git', ['archive', '--format=tar', `--output=${archive}`, sha]);
    mkdirSync(source);
    run('tar', ['-xf', archive, '-C', source], directory);
    // export-ignore must never silently remove sources from the Cargo graph at the cut.
    const files = run('git', ['ls-tree', '-r', '--name-only', '-z', sha])
      .split('\0')
      .filter(Boolean);
    for (const file of files) {
      try {
        lstatSync(join(source, file));
      } catch (error) {
        throw new Error(`Tracked release source omitted from export: ${file}`, { cause: error });
      }
    }
    return read(source);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

/** One exact-ref export and one offline Cargo read for cut, publication and Project consumers. */
export function readReleaseSourceAtRef(sha, { warm = false, ...options } = {}) {
  return withReleaseSourceAtRef(
    sha,
    (root) => {
      if (warm) {
        // Cargo fetch resolves/downloads locked dependencies; it never runs captured build scripts.
        (options.execute ?? runPackedCommand)(
          'cargo',
          [
            '+1.97.0',
            'fetch',
            '--quiet',
            '--locked',
            '--manifest-path',
            join(root, 'rust/Cargo.toml'),
          ],
          { cwd: root, env: process.env, timeoutMs: 120_000 }
        );
      }
      return {
        map: parseComponentMap(readFileSync(join(root, '.github/components.json'), 'utf8'), {
          historical: true,
        }),
        workspace: readCargoWorkspace(root),
      };
    },
    options
  );
}
