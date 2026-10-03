import { mkdtempSync, mkdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vite-plus/test';
import { readCargoWorkspace } from '../../scripts/cargo-workspace.mjs';

const { runPackedCommand } = (await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
)) as {
  runPackedCommand: (
    command: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }
  ) => string;
};

describe('Cargo workspace reader', () => {
  it('preserves resolved facts and distinguishes aliased, build and dev workspace edges with cycle-safe closure', () => {
    const root = mkdtempSync(join(tmpdir(), 'tmt-cargo-reader-'));
    const names = ['product', 'normal', 'build', 'dev'];
    try {
      const packages = names.map((name, index) => {
        const dir = join(root, 'rust', name);
        mkdirSync(dir, { recursive: true });
        return {
          id: name,
          name,
          version: '1.2.3',
          manifest_path: join(dir, 'Cargo.toml'),
          targets: [{ name, kind: index ? ['lib'] : ['bin'] }],
          metadata: { dist: { dist: index === 0, 'install-path': 'CARGO_HOME' } },
          dependencies:
            index === 0
              ? names.slice(1).map((dependency, i) => ({
                  name: dependency,
                  rename: `alias${i}`,
                  path: join(root, 'rust', dependency),
                  kind: [null, 'build', 'dev'][i],
                }))
              : index === 1
                ? [{ name: 'product', path: join(root, 'rust', 'product'), kind: null }]
                : [],
        };
      });
      const runner = vi.fn(() => JSON.stringify({ workspace_members: names, packages }));
      const workspace = readCargoWorkspace(root, { runner });
      expect(runner).toHaveBeenCalledWith(
        'cargo',
        [
          'metadata',
          '--quiet',
          '--format-version',
          '1',
          '--offline',
          '--locked',
          '--manifest-path',
          join(root, 'rust/Cargo.toml'),
        ],
        expect.objectContaining({ cwd: join(root, 'rust'), timeoutMs: 60_000 })
      );
      expect(workspace.packages[0]).toMatchObject({
        name: 'product',
        version: '1.2.3',
        manifest: 'rust/product/Cargo.toml',
        dir: 'rust/product',
        binTargets: ['product'],
        distMetadata: { dist: true },
        dependencies: { normal: ['normal'], build: ['build'], dev: ['dev'] },
      });
      expect(
        workspace
          .closure('product', ['normal', 'build'])
          .map((p) => p.name)
          .sort()
      ).toEqual(['build', 'normal', 'product']);
      expect(
        workspace
          .closure('product', ['dev'])
          .map((p) => p.name)
          .sort()
      ).toEqual(['dev', 'product']);
      expect(() => workspace.closure('missing', ['normal'])).toThrow('Unknown Cargo');
      expect(() => workspace.closure('product', [])).toThrow('Invalid Cargo closure');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it('suppresses Cargo lock progress while preserving strict status and diagnostic checks', () => {
    const root = mkdtempSync(join(tmpdir(), 'tmt-cargo-reader-'));
    try {
      mkdirSync(join(root, 'rust'));
      const metadata = JSON.stringify({
        workspace_members: ['product'],
        packages: [
          {
            id: 'product',
            name: 'product',
            version: '1.2.3',
            manifest_path: join(root, 'rust/Cargo.toml'),
            targets: [],
            dependencies: [],
          },
        ],
      });
      const read = (diagnostic = '', status = 0) =>
        readCargoWorkspace(root, {
          runner: (
            _executable: string,
            args: string[],
            options: Parameters<typeof runPackedCommand>[2]
          ) =>
            runPackedCommand(
              process.execPath,
              [
                '-e',
                // Pinned Cargo suppresses this progress message under --quiet, including during a wait.
                `if (!process.argv.includes('--quiet')) process.stderr.write(${JSON.stringify('    Blocking waiting for file lock on package cache\n')});
           process.stderr.write(${JSON.stringify(diagnostic)});
           process.stdout.write(${JSON.stringify(metadata)});
           process.exit(${status});`,
                '--',
                ...args,
              ],
              options
            ),
        });
      expect(read().packages[0].version).toBe('1.2.3');
      expect(() => read('error: offline dependency unavailable\n', 1)).toThrow('exited 1');
      expect(() => read('unexpected Cargo diagnostic\n')).toThrow('unexpected diagnostics');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
  it('propagates Cargo failure instead of deriving an empty graph', () => {
    expect(() =>
      readCargoWorkspace('/private/tmp', {
        runner: () => {
          throw new Error('offline dependency unavailable');
        },
      })
    ).toThrow('offline dependency unavailable');
  });
});
