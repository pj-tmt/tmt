import { beforeEach, expect, it, vi } from 'vite-plus/test';
import type { CargoWorkspace } from '../../scripts/cargo-workspace.mjs';

const { readCargoWorkspace } = vi.hoisted(() => ({
  readCargoWorkspace: vi.fn((): CargoWorkspace => ({ packages: [], closure: () => [] })),
}));

vi.mock('../../scripts/cargo-workspace.mjs', () => ({ readCargoWorkspace }));

function workspace(remoteVersion = '0.2.0-alpha.7'): CargoWorkspace {
  return {
    packages: [
      {
        name: 'other',
        version: '1.0.0',
        manifest: 'rust/crates/other/Cargo.toml',
        dir: 'rust/crates/other',
        manifestPath: '/fixture/rust/crates/other/Cargo.toml',
        binTargets: [],
        distMetadata: {},
        dependencies: { normal: [], build: [], dev: [] },
      },
      {
        name: 'tmt-cli',
        version: '5.0.0-alpha.9',
        manifest: 'rust/crates/tmt-cli/Cargo.toml',
        dir: 'rust/crates/tmt-cli',
        manifestPath: '/fixture/Cargo.toml',
        binTargets: ['fixture'],
        distMetadata: {},
        dependencies: { normal: [], build: [], dev: [] },
      },
      {
        name: 'tmt-remote',
        version: remoteVersion,
        manifest: 'extensions/tmt-remote/rust/tmt-remote/Cargo.toml',
        dir: 'extensions/tmt-remote/rust/tmt-remote',
        manifestPath: '/fixture/Cargo.toml',
        binTargets: ['fixture'],
        distMetadata: {},
        dependencies: { normal: [], build: [], dev: [] },
      },
    ],
    closure: () => [],
  };
}

beforeEach(() => {
  vi.resetModules();
  readCargoWorkspace.mockReset().mockReturnValue(workspace());
});

it('selects independently versioned CLI and Remote crates with one metadata read', async () => {
  const { workspaceVersion } = await import('../support/workspace-version.js');
  expect(readCargoWorkspace).not.toHaveBeenCalled();
  expect(workspaceVersion('tmt-cli')).toBe('5.0.0-alpha.9');
  expect(workspaceVersion('tmt-remote')).toBe('0.2.0-alpha.7');
  expect(workspaceVersion('tmt-remote')).toBe('0.2.0-alpha.7');
  expect(workspaceVersion('tmt-cli')).toBe('5.0.0-alpha.9');
  expect(workspaceVersion('other')).toBe('1.0.0');
  expect(readCargoWorkspace).toHaveBeenCalledTimes(1);
});

it('picks up an Remote version bump without changing the CLI expectation', async () => {
  readCargoWorkspace.mockReturnValue(workspace('0.2.0-alpha.8'));
  const { workspaceVersion } = await import('../support/workspace-version.js');
  expect(workspaceVersion('tmt-remote')).toBe('0.2.0-alpha.8');
  expect(workspaceVersion('tmt-cli')).toBe('5.0.0-alpha.9');
  expect(readCargoWorkspace).toHaveBeenCalledTimes(1);
});

it('names a missing crate and retains the resolved metadata for valid callers', async () => {
  const { workspaceVersion } = await import('../support/workspace-version.js');
  expect(() => workspaceVersion('missing')).toThrow('Cargo workspace has no missing crate.');
  expect(workspaceVersion('tmt-cli')).toBe('5.0.0-alpha.9');
  expect(readCargoWorkspace).toHaveBeenCalledTimes(1);
});
