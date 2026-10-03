import { beforeEach, expect, it, vi } from 'vite-plus/test';
import type { Workspace } from '../../scripts/cargo-workspace.mjs';

const { readWorkspace } = vi.hoisted(() => ({
  readWorkspace: vi.fn((): Workspace => ({ crates: [], lockNames: new Set(), files: [] })),
}));

vi.mock('../../scripts/cargo-workspace.mjs', () => ({ readWorkspace }));

function workspace(officeVersion = '0.2.0-alpha.7'): Workspace {
  return {
    crates: [
      {
        name: 'other',
        version: '1.0.0',
        manifest: 'rust/crates/other/Cargo.toml',
        dir: 'rust/crates/other',
        inheritsVersion: true,
        hasBinary: false,
        dependencies: [],
      },
      {
        name: 'tmt-cli',
        version: '5.0.0-alpha.9',
        manifest: 'rust/crates/tmt-cli/Cargo.toml',
        dir: 'rust/crates/tmt-cli',
        inheritsVersion: false,
        hasBinary: true,
        dependencies: [],
      },
      {
        name: 'tmt-office',
        version: officeVersion,
        manifest: 'extensions/tmt-office/rust/tmt-office/Cargo.toml',
        dir: 'extensions/tmt-office/rust/tmt-office',
        inheritsVersion: false,
        hasBinary: true,
        dist: false,
        dependencies: [],
      },
    ],
    lockNames: new Set(['other', 'tmt-cli', 'tmt-office']),
    files: [],
  };
}

beforeEach(() => {
  vi.resetModules();
  readWorkspace.mockReset().mockReturnValue(workspace());
});

it('selects independently versioned CLI and parked Office crates with one metadata read', async () => {
  const { workspaceVersion } = await import('../support/workspace-version.js');
  expect(readWorkspace).not.toHaveBeenCalled();
  expect(workspaceVersion('tmt-cli')).toBe('5.0.0-alpha.9');
  expect(workspaceVersion('tmt-office')).toBe('0.2.0-alpha.7');
  expect(workspaceVersion('tmt-office')).toBe('0.2.0-alpha.7');
  expect(workspaceVersion('tmt-cli')).toBe('5.0.0-alpha.9');
  expect(workspaceVersion('other')).toBe('1.0.0');
  expect(readWorkspace).toHaveBeenCalledTimes(1);
});

it('picks up an Office version bump without changing the CLI expectation', async () => {
  readWorkspace.mockReturnValue(workspace('0.2.0-alpha.8'));
  const { workspaceVersion } = await import('../support/workspace-version.js');
  expect(workspaceVersion('tmt-office')).toBe('0.2.0-alpha.8');
  expect(workspaceVersion('tmt-cli')).toBe('5.0.0-alpha.9');
  expect(readWorkspace).toHaveBeenCalledTimes(1);
});

it('names a missing crate and retains the resolved metadata for valid callers', async () => {
  const { workspaceVersion } = await import('../support/workspace-version.js');
  expect(() => workspaceVersion('missing')).toThrow('Cargo workspace has no missing crate.');
  expect(workspaceVersion('tmt-cli')).toBe('5.0.0-alpha.9');
  expect(readWorkspace).toHaveBeenCalledTimes(1);
});
