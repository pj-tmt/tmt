import { expect, it, vi } from 'vitest';
import type { Workspace } from '../../scripts/release-please-config.mjs';
import { workspaceVersion } from '../support/workspace-version.js';

const { readWorkspace } = vi.hoisted(() => ({
  readWorkspace: vi.fn(
    () =>
      ({
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
            hasBinary: false,
            dependencies: [],
          },
        ],
        lockNames: new Set(['other', 'tmt-cli']),
        files: [],
      }) satisfies Workspace
  ),
}));

vi.mock('../../scripts/release-please-config.mjs', () => ({ readWorkspace }));

it('uses the resolved CLI version regardless of inheritance and reads metadata only once', () => {
  expect(readWorkspace).not.toHaveBeenCalled();
  expect(workspaceVersion()).toBe('5.0.0-alpha.9');
  expect(workspaceVersion()).toBe('5.0.0-alpha.9');
  expect(readWorkspace).toHaveBeenCalledTimes(1);
});
