import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import {
  CLI_NAMESPACE,
  LEGACY_CLI_NAMESPACE,
  readCliLayout,
} from '../../scripts/native-install-layout.mjs';

const oldId = '10000000-0000-4000-8000-000000000001';
const newId = '10000000-0000-4000-8000-000000000002';

function withLayouts(run: (prefix: string) => void) {
  const world = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-layout-')));
  const prefix = path.join(world, 'prefix with spaces');
  fs.mkdirSync(path.join(prefix, 'bin'), { recursive: true });
  try {
    run(prefix);
  } finally {
    fs.rmSync(world, { recursive: true, force: true });
  }
}

function activation(prefix: string, namespace: string, id: string) {
  const root = path.join(prefix, namespace);
  const release = path.join(root, 'releases', id);
  fs.mkdirSync(release, { recursive: true });
  fs.writeFileSync(path.join(release, 'tmt'), 'verified fixture bytes');
  const receipt = {
    schema_version: 1,
    prefix,
    release_id: id,
    version: id === oldId ? '5.0.0-alpha.1' : '5.0.0-alpha.2',
    channel: 'alpha',
  };
  fs.writeFileSync(path.join(release, 'receipt.json'), JSON.stringify(receipt));
  fs.symlinkSync(`releases/${id}`, path.join(root, 'current'));
  return { root, release, receipt };
}

function select(prefix: string, namespace: string) {
  fs.symlinkSync(`../${namespace}/current/tmt`, path.join(prefix, 'bin/tmt'));
}

describe('managed CLI active-layout evidence', () => {
  it.each([LEGACY_CLI_NAMESPACE, CLI_NAMESPACE])('reads the selected %s layout', (namespace) => {
    withLayouts((prefix) => {
      const expected = activation(prefix, namespace, oldId);
      select(prefix, namespace);
      expect(readCliLayout(prefix)).toEqual({
        prefix,
        namespace,
        root: expected.root,
        current: path.join(expected.root, 'current'),
        release: expected.release,
        executable: path.join(expected.release, 'tmt'),
        receipt: expected.receipt,
      });
    });
  });

  it('follows bin/tmt with differing rollback activations, then with the former prefix retired', () => {
    withLayouts((prefix) => {
      const former = activation(prefix, LEGACY_CLI_NAMESPACE, oldId);
      const active = activation(prefix, CLI_NAMESPACE, newId);
      select(prefix, LEGACY_CLI_NAMESPACE);
      expect(readCliLayout(prefix).receipt).toEqual(former.receipt);
      fs.unlinkSync(path.join(prefix, 'bin/tmt'));
      select(prefix, CLI_NAMESPACE);
      expect(readCliLayout(prefix).receipt).toEqual(active.receipt);
      fs.rmSync(former.root, { recursive: true });
      expect(readCliLayout(prefix).receipt).toEqual(active.receipt);
    });
  });

  it('refuses a damaged selected new receipt instead of falling back to a valid former one', () => {
    withLayouts((prefix) => {
      activation(prefix, LEGACY_CLI_NAMESPACE, oldId);
      const active = activation(prefix, CLI_NAMESPACE, newId);
      select(prefix, CLI_NAMESPACE);
      fs.writeFileSync(path.join(active.release, 'receipt.json'), '{truncated');
      expect(() => readCliLayout(prefix)).toThrow();
    });
  });

  it.each([
    'foreign-command',
    'outside-link',
    'unknown-namespace',
    'outside-namespace',
    'outside-current',
    'different-executable',
    'linked-receipt',
  ])('refuses %s before accepting receipt evidence', (kind) => {
    withLayouts((prefix) => {
      const active = activation(prefix, CLI_NAMESPACE, newId);
      select(prefix, CLI_NAMESPACE);
      const command = path.join(prefix, 'bin/tmt');
      let message = '';
      if (kind === 'foreign-command') {
        fs.unlinkSync(command);
        fs.writeFileSync(command, 'foreign');
        message = 'managed link';
      } else if (kind === 'outside-link' || kind === 'unknown-namespace') {
        fs.unlinkSync(command);
        fs.symlinkSync(
          kind === 'outside-link' ? '/outside/tmt' : '../lib/unknown/current/tmt',
          command
        );
        message = kind === 'outside-link' ? 'escapes owned prefix' : 'unknown namespace';
      } else if (kind === 'outside-namespace') {
        fs.rmSync(active.root, { recursive: true });
        fs.symlinkSync(path.dirname(prefix), active.root);
        message = 'namespace escapes owned prefix';
      } else if (kind === 'outside-current') {
        fs.unlinkSync(path.join(active.root, 'current'));
        fs.symlinkSync(path.dirname(prefix), path.join(active.root, 'current'));
        message = 'receipt escapes owned prefix';
      } else if (kind === 'different-executable') {
        fs.unlinkSync(path.join(active.release, 'tmt'));
        const other = path.join(prefix, 'other');
        fs.writeFileSync(other, 'other bytes');
        fs.symlinkSync(other, path.join(active.release, 'tmt'));
        message = 'executable/current mismatch';
      } else {
        const receipt = path.join(active.release, 'receipt.json');
        fs.unlinkSync(receipt);
        const other = path.join(prefix, 'other.json');
        fs.writeFileSync(other, JSON.stringify(active.receipt));
        fs.symlinkSync(other, receipt);
        message = 'receipt must be a regular file';
      }
      expect(() => readCliLayout(prefix)).toThrow(message);
    });
  });

  it.each([
    { prefix: '/another/prefix' },
    { release_id: oldId },
    { schema_version: 2 },
    { version: null },
    { channel: null },
  ])('binds the receipt identity to the selected path: %j', (change) => {
    withLayouts((prefix) => {
      const active = activation(prefix, CLI_NAMESPACE, newId);
      select(prefix, CLI_NAMESPACE);
      fs.writeFileSync(
        path.join(active.release, 'receipt.json'),
        JSON.stringify({ ...active.receipt, ...change })
      );
      expect(() => readCliLayout(prefix)).toThrow('receipt does not match');
    });
  });
});
