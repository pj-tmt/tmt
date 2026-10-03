import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

const {
  HANDOFF_PROBE,
  hasInstallerHandoff,
  installerProtocol,
  requireInstallerProtocol,
  requireLegacyInventoryError,
} = await import('../../scripts/native-upgrade-proof.mjs');
const options = { cwd: '/fixture', env: {} };
const legacy = () => {
  const error = new assert.AssertionError({ message: 'nonzero probe' });
  error.cause = {
    status: 1,
    stderr: '',
    stdout: JSON.stringify({
      error: {
        code: 'USAGE_ERROR',
        message:
          "error: unexpected argument '--handoff-version' found\n\nUsage: tmt __native-install",
      },
    }),
  };
  throw error;
};

describe('native installer protocol and legacy recovery controls', () => {
  it('uses candidate-source applicability, not a failed probe or mutable release version', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'installer-source-'));
    const directory = path.join(root, 'rust/crates/tmt-adapters/src/native_install');
    const sourceRoot = pathToFileURL(`${root}/`);
    try {
      expect(() => hasInstallerHandoff(sourceRoot)).toThrow();
      fs.mkdirSync(directory, { recursive: true });
      expect(hasInstallerHandoff(sourceRoot)).toBe(false);
      for (const content of ['', 'pub const VERSION: u32 = 2;']) {
        fs.writeFileSync(path.join(directory, 'handoff.rs'), content);
        expect(() => hasInstallerHandoff(sourceRoot)).toThrow('protocol 1');
      }
      fs.writeFileSync(path.join(directory, 'handoff.rs'), 'pub const VERSION: u32 = 1;\n');
      expect(hasInstallerHandoff(sourceRoot)).toBe(true);
      // Source applicability never converts a legacy candidate response into a passing probe.
      expect(() => requireInstallerProtocol('/candidate/tmt', options, legacy)).toThrow(
        'before publication'
      );
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
  it('probes the exact contract and requires it from a candidate', () => {
    const execute = (command: string, args: string[]) => {
      expect(command).toBe('/candidate/tmt');
      expect(args).toEqual(['__native-install', '--handoff-version', '1', '--probe', '--json']);
      return '{"protocol":1}\n';
    };
    expect(HANDOFF_PROBE).toHaveLength(5);
    expect(installerProtocol('/candidate/tmt', options, execute)).toBe('handoff');
    expect(() => requireInstallerProtocol('/candidate/tmt', options, execute)).not.toThrow();
    expect(installerProtocol('/old/tmt', options, legacy)).toBe('legacy');
    expect(() => requireInstallerProtocol('/old/tmt', options, legacy)).toThrow(
      'before publication'
    );
  });

  it.each([
    '{"protocol":2}',
    '{"protocol":1,"extra":true}',
    '{}',
    'invalid',
    '{"protocol":1}\nprogress',
    ' {"protocol":1}',
    '{"protocol":1}\n\n',
  ])('refuses malformed or unsupported candidate response %s', (output) => {
    expect(() => requireInstallerProtocol('/candidate/tmt', options, () => output)).toThrow(
      'exactly'
    );
  });

  it.each([
    { status: 1, stdout: '{"error":{"code":"IO_ERROR","message":"disk error"}}', stderr: '' },
    {
      status: 1,
      stdout: '{"error":{"code":"USAGE_ERROR","message":"another argument failed"}}',
      stderr: '',
    },
    { status: 1, stdout: 'not JSON', stderr: '' },
    { status: 1, stdout: '', stderr: 'permission denied' },
    { status: 137, stdout: '', stderr: '' },
  ])('never downgrades an unrelated probe failure to legacy', (cause) => {
    expect(() =>
      installerProtocol('/source/tmt', options, () => {
        const error = new Error('probe failed', { cause });
        throw error;
      })
    ).toThrow();
  });

  it('requires the actual historical error, not any failed install', () => {
    expect(() =>
      requireLegacyInventoryError({
        error: {
          code: 'NATIVE_INSTALL_FAILED',
          message: 'Unexpected native archive asset inventory.',
        },
      })
    ).not.toThrow();
    for (const error of [
      { code: 'NATIVE_INSTALL_FAILED', message: 'checksum mismatch' },
      {
        code: 'NATIVE_UPGRADE_INSTALLER_UNSUPPORTED',
        message: 'Unexpected native archive asset inventory.',
      },
      {
        code: 'NATIVE_INSTALL_FAILED',
        message: 'Unexpected native archive asset inventory. additional detail',
      },
    ])
      expect(() => requireLegacyInventoryError({ error })).toThrow();
  });
});
