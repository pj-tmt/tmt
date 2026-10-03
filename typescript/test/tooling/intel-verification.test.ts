import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';

const { runPackedCommand } = (await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
)) as {
  runPackedCommand: (
    command: string,
    args: string[],
    options: {
      cwd: string;
      env: NodeJS.ProcessEnv;
      expectedStatus: number;
      timeoutMs: number;
    }
  ) => string;
};

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const read = (name: string) => readFileSync(path.join(repository, name), 'utf8');
const wrapper = read('scripts/run-native-verification.sh');

describe('target-specific verification process tree', () => {
  it('runs the complete script with the Intel preference and stops before it for an arm64 Node', () => {
    const root = mkdtempSync(path.join(os.tmpdir(), 'intel-verification-'));
    try {
      const arch = path.join(root, 'arch');
      const uname = path.join(root, 'uname');
      writeExecutable(
        arch,
        '#!/bin/sh\nset -eu\nprintf "%s\\n" "$*" > "$ROOT/arch-call"\ntest "$1" = -x86_64\nshift\nexport TMT_FIXTURE_PREFERENCE=x86_64\nexec "$@"\n',
        0o700
      );
      writeExecutable(uname, '#!/bin/sh\ntest "$1" = -m\nprintf x86_64\n', 0o700);
      writeExecutable(
        path.join(root, 'node'),
        '#!/bin/sh\ntest "$*" = "-p process.arch"\nprintf "%s" "$NODE_ARCH"\n',
        0o700
      );
      const fixture = path.join(root, 'verification.sh');
      writeFileSync(
        fixture,
        wrapper.replace('/usr/bin/arch', arch).replace('/usr/bin/uname', uname)
      );
      const input = ': > "$ROOT/executed"\n/bin/sh -c \'printf "%s" "$TMT_FIXTURE_PREFERENCE"\'\n';
      const body = path.join(root, 'body.bash');
      writeFileSync(body, input);
      const run = (architecture: string, expectedStatus: number) =>
        runPackedCommand(
          '/bin/sh',
          [
            '-c',
            'exec /bin/sh "$1" "$2" < "$3"',
            'verification',
            fixture,
            'x86_64-apple-darwin',
            body,
          ],
          {
            cwd: root,
            expectedStatus,
            timeoutMs: 5000,
            env: {
              ROOT: root,
              NODE_ARCH: architecture,
              PATH: `${root}:/usr/bin:/bin`,
              TMPDIR: root,
            },
          }
        );
      expect(run('arm64', 1)).toBe('');
      expect(existsSync(path.join(root, 'executed'))).toBe(false);
      expect(
        readdirSync(root).filter((name) => name.startsWith('tmt-native-verification.'))
      ).toEqual([]);
      expect(run('x64', 0)).toBe('x86_64');
      expect(existsSync(path.join(root, 'executed'))).toBe(true);
      expect(
        readdirSync(root).filter((name) => name.startsWith('tmt-native-verification.'))
      ).toEqual([]);
      expect(readFileSync(path.join(root, 'arch-call'), 'utf8')).toContain(
        '-x86_64 /bin/bash --noprofile --norc -euo pipefail'
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it('retains stdin and pipe failure for other targets, and rejects a missing target', () => {
    const root = mkdtempSync(path.join(os.tmpdir(), 'native-verification-'));
    try {
      const body = path.join(root, 'body.bash');
      const run = (target: string, input: string, expectedStatus = 0) => {
        writeFileSync(body, input);
        return runPackedCommand(
          '/bin/sh',
          [
            '-c',
            'exec /bin/sh "$1" "$2" < "$3"',
            'verification',
            path.join(repository, 'scripts/run-native-verification.sh'),
            target,
            body,
          ],
          {
            cwd: root,
            env: process.env,
            expectedStatus,
            timeoutMs: 5000,
          }
        );
      };
      expect(run('aarch64-apple-darwin', 'printf native')).toBe('native');
      expect(run('x86_64-unknown-linux-musl', 'false | true', 1)).toBe('');
      expect(() => run('', 'printf should-not-run')).toThrow('Usage:');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

describe('Intel workflow coverage', () => {
  it('keeps weekly/manual real Intel public detection and upgrade, with a PR trigger only on itself', () => {
    const workflow = read('.github/workflows/native-intel.yml');
    expect(workflow).toMatch(/schedule:\n {4}- cron: '[^']+'\n {2}workflow_dispatch:/);
    expect(workflow).toMatch(
      /pull_request:\n {4}paths:\n {6}- \.github\/workflows\/native-intel.yml\n\npermissions:/
    );
    expect(workflow).toContain('runs-on: macos-15-intel');
    expect(workflow).toContain('test "$RUNNER_ARCH" = X64');
    expect(workflow).toContain('test "$(uname -m)" = x86_64');
    expect(workflow).toContain('cargo build --locked --release --target "$TARGET" -p tmt-cli');
    expect(workflow).toContain('node typescript/scripts/verify-native-runtime.mjs');
    expect(workflow).toContain('node typescript/scripts/verify-public-install.mjs --product cli');
    expect(workflow).toContain('releases/latest');
    expect(workflow).toMatch(/^permissions:\n {2}contents: read$/m);
    expect(workflow).not.toMatch(/: write|gh release|release-publish|uses: .*native-release/);
    const publicStep = workflow
      .slice(workflow.indexOf('      - name: Prove public installer'))
      .split(/\n  [a-z][a-z0-9-]*:\n/)[0];
    expect(publicStep).not.toMatch(/GH_TOKEN|GITHUB_TOKEN|secrets\./);
  });

  it('moves all ordinary Intel rows to arm64 with an x64 Node and a whole-step execution preference', () => {
    for (const name of [
      'ci.yml',
      'native-release-bundle.yml',
      'native-release-upgrade.yml',
      'native-release-smoke.yml',
    ]) {
      const workflow = read(`.github/workflows/${name}`);
      expect(workflow, name).not.toContain('macos-15-intel');
      expect(workflow, name).toMatch(
        /architecture: \$\{\{ matrix.target == 'x86_64-apple-darwin' && 'x64' \|\| '' \}\}/
      );
      expect(workflow, name).toContain('scripts/run-native-verification.sh');
      expect(workflow, name).toContain('warm-xcrun');
    }
    expect(read('.github/actions/setup-tooling/action.yml')).toContain(
      'architecture: ${{ inputs.architecture }}'
    );
  });

  it('fails closed for candidate checkouts without Rosetta tooling and names the owner remedy', () => {
    for (const name of ['native-release-bundle.yml', 'native-release-upgrade.yml']) {
      const workflow = read(`.github/workflows/${name}`);
      const guard = workflow.indexOf('      - name: Require candidate Rosetta tooling');
      expect(guard).toBeGreaterThan(0);
      expect(guard).toBeLessThan(workflow.indexOf('      - name: Set up Node.js and pnpm', guard));
      expect(workflow).toContain("if: matrix.target == 'x86_64-apple-darwin'");
      expect(workflow).toContain('if [ ! -x scripts/run-native-verification.sh ]; then');
      expect(workflow).toContain(
        "::error::candidate predates #547 Rosetta tooling; the owner reruns with native-release rerun=<tag>'\n            exit 1"
      );
    }
  });
});
