import { spawnSync } from 'node:child_process';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
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
  it.each([
    {
      name: 'arm64 Node',
      nodeArch: 'arm64',
      hostArch: 'x86_64',
      nodeExit: '0',
      hostExit: '0',
      diagnostic: 'Node process.arch expected x64, got arm64',
    },
    {
      name: 'arm64 uname',
      nodeArch: 'x64',
      hostArch: 'arm64',
      nodeExit: '0',
      hostExit: '0',
      diagnostic: 'uname -m expected x86_64, got arm64',
    },
    {
      name: 'failed Node probe',
      nodeArch: 'x64',
      hostArch: 'x86_64',
      nodeExit: '1',
      hostExit: '0',
      diagnostic: 'node -p process.arch could not run',
    },
    {
      name: 'failed uname probe',
      nodeArch: 'x64',
      hostArch: 'x86_64',
      nodeExit: '0',
      hostExit: '1',
      diagnostic: 'uname -m could not run',
    },
  ])(
    'names the failed $name check, stops before input and cleans up; valid Intel executes',
    (scenario) => {
      const root = mkdtempSync(path.join(os.tmpdir(), 'intel-verification-'));
      try {
        const arch = path.join(root, 'arch');
        const uname = path.join(root, 'uname');
        writeExecutable(
          arch,
          '#!/bin/sh\nset -eu\nprintf "%s\\n" "$*" > "$ROOT/arch-call"\ntest "$1" = -x86_64\nshift\nexport TMT_FIXTURE_PREFERENCE=x86_64\nexec "$@"\n',
          0o700
        );
        writeExecutable(
          uname,
          '#!/bin/sh\ntest "$1" = -m\nprintf \'%s\' "$HOST_ARCH"\nexit "$HOST_EXIT"\n',
          0o700
        );
        writeExecutable(
          path.join(root, 'node'),
          '#!/bin/sh\ntest "$*" = "-p process.arch"\nprintf "%s" "$NODE_ARCH"\nexit "$NODE_EXIT"\n',
          0o700
        );
        const fixture = path.join(root, 'verification.sh');
        writeFileSync(
          fixture,
          wrapper.replace('/usr/bin/arch', arch).replace('/usr/bin/uname', uname)
        );
        const input =
          ': > "$ROOT/executed"\n/bin/sh -c \'printf "%s" "$TMT_FIXTURE_PREFERENCE"\'\n';
        const run = (valid: boolean) =>
          spawnSync('/bin/sh', [fixture, 'x86_64-apple-darwin'], {
            cwd: root,
            encoding: 'utf8',
            input,
            timeout: 5000,
            env: {
              ROOT: root,
              NODE_ARCH: valid ? 'x64' : scenario.nodeArch,
              HOST_ARCH: valid ? 'x86_64' : scenario.hostArch,
              NODE_EXIT: valid ? '0' : scenario.nodeExit,
              HOST_EXIT: valid ? '0' : scenario.hostExit,
              PATH: `${root}:/usr/bin:/bin`,
              TMPDIR: root,
            },
          });
        const rejected = run(false);
        expect(rejected.error).toBeUndefined();
        expect(rejected.signal).toBeNull();
        expect(rejected.status).toBe(1);
        expect(rejected.stdout).toBe('');
        expect(rejected.stderr).toContain(
          `Intel verification host check failed: ${scenario.diagnostic}`
        );
        if (scenario.name === 'arm64 Node')
          expect(rejected.stderr).toContain(`node: ${path.join(root, 'node')}`);
        expect(existsSync(path.join(root, 'executed'))).toBe(false);
        expect(
          readdirSync(root).filter((name) => name.startsWith('tmt-native-verification.'))
        ).toEqual([]);
        const accepted = run(true);
        expect(accepted.error).toBeUndefined();
        expect(accepted.signal).toBeNull();
        expect(accepted.status).toBe(0);
        expect(accepted.stderr).toBe('');
        expect(accepted.stdout).toBe('x86_64');
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
    }
  );

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
  it.each(['false', 'true'])(
    'retains upgrade and adapter arguments with current tooling=%s',
    (current) => {
      const workflow = read('.github/workflows/native-release-upgrade-prove.yml');
      const root = mkdtempSync(path.join(os.tmpdir(), 'intel-upgrade-arguments-'));
      try {
        const record = path.join(root, 'arguments');
        writeExecutable(
          path.join(root, 'node'),
          '#!/bin/sh\nprintf \'%s\\n\' "$@" > "$RECORD_FILE"\n'
        );
        mkdirSync(path.join(root, 'scripts'));
        writeExecutable(path.join(root, 'scripts/run-native-verification.sh'), wrapper);
        for (const [name, mode] of [
          ['Prove last-published and declared-floor installation and bootstrap recovery', 'prove'],
          ['Prove the real-archive CLI upgrade adapter', 'acceptance'],
        ]) {
          const step = workflow.split(`      - name: ${name}\n`)[1].split('\n      - name:')[0];
          const body = step.split('        run: |\n')[1].replace(/^          /gm, '');
          const script = path.join(root, 'step.bash');
          writeFileSync(script, body);
          runPackedCommand('/bin/bash', ['-euo', 'pipefail', script], {
            cwd: repository,
            env: {
              PATH: `${root}:/usr/bin:/bin`,
              RECORD_FILE: record,
              PRODUCT: 'cli',
              RELEASE_TAG: 'v5.0.0-alpha.47',
              TARGET: 'aarch64-apple-darwin',
              CURRENT_TOOLING: current,
              GITHUB_WORKSPACE: root,
              RUNNER_TEMP: root,
              GITHUB_STEP_SUMMARY: path.join(root, 'summary'),
            },
            expectedStatus: 0,
            timeoutMs: 5000,
          });
          expect(readFileSync(record, 'utf8').trim().split('\n')).toEqual([
            current === 'true'
              ? path.join(root, 'typescript/scripts/release-upgrade.mjs')
              : './typescript/scripts/release-upgrade.mjs',
            mode,
            '--product',
            'cli',
            '--tag',
            'v5.0.0-alpha.47',
            '--target',
            'aarch64-apple-darwin',
            '--directory',
            path.join(root, 'upgrade'),
            ...(mode === 'prove'
              ? [
                  '--skill',
                  current === 'true'
                    ? path.join(root, 'release-source/skills/tmux-team/SKILL.md')
                    : 'skills/tmux-team/SKILL.md',
                ]
              : []),
            ...(current === 'true' ? ['--source-root', path.join(root, 'release-source')] : []),
          ]);
        }
      } finally {
        rmSync(root, { recursive: true, force: true });
      }
    }
  );

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
    expect(publicStep).toContain('GITHUB_TOKEN: ${{ github.token }}');
    expect(publicStep).not.toMatch(/GH_TOKEN:|secrets\./);
    expect(workflow).not.toContain('infrastructure');
  });

  it('moves all ordinary Intel rows to arm64 with an x64 Node and a whole-step execution preference', () => {
    for (const name of [
      'ci.yml',
      'native-release-prepare.yml',
      'native-release-upgrade-prove.yml',
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
    const publicSmoke = read('.github/actions/public-install-smoke/action.yml');
    expect(publicSmoke).toContain(
      "architecture: ${{ inputs.target == 'x86_64-apple-darwin' && 'x64' || '' }}"
    );
    expect(publicSmoke).toContain('scripts/run-native-verification.sh "$TARGET"');
    expect(publicSmoke).toContain('warm-xcrun');
    const driverSetup = publicSmoke
      .split('    - name: Set up driver archive verification tooling')[1]
      .split('    - name: Install driver archive verification dependencies')[0];
    expect(driverSetup).toContain("if: inputs.product == 'driver-herdr'");
    expect(driverSetup).toContain('uses: ./.github/actions/setup-tooling');
    expect(driverSetup).toContain(
      "architecture: ${{ inputs.target == 'x86_64-apple-darwin' && 'x64' || '' }}"
    );
    for (const name of ['native-release-smoke.yml']) {
      expect(read(`.github/workflows/${name}`), name).not.toContain('macos-15-intel');
      expect(read(`.github/workflows/${name}`), name).toContain(
        'uses: ./.github/actions/public-install-smoke'
      );
    }
    const upgrade = read('.github/workflows/native-release-upgrade-prove.yml');
    const acceptance = upgrade.split('      - name: Prove the real-archive CLI upgrade adapter')[1];
    expect(acceptance).toContain('verification="$PWD/scripts/run-native-verification.sh"');
    expect(acceptance).toContain(
      'verification="$GITHUB_WORKSPACE/scripts/run-native-verification.sh"'
    );
    expect(acceptance).toContain('"$verification" "$TARGET"');
    expect(acceptance).toContain('release-upgrade.mjs" acceptance');
  });

  it('fails closed for candidate checkouts without Rosetta tooling and names the owner remedy', () => {
    for (const name of ['native-release-prepare.yml', 'native-release-upgrade-prove.yml']) {
      const workflow = read(`.github/workflows/${name}`);
      const guard = workflow.indexOf('      - name: Require candidate Rosetta tooling');
      expect(guard).toBeGreaterThan(0);
      expect(guard).toBeLessThan(workflow.indexOf('      - name: Set up Node.js and pnpm', guard));
      expect(workflow).toContain(
        name === 'native-release-upgrade-prove.yml'
          ? "if: ${{ matrix.target == 'x86_64-apple-darwin' && !inputs.current-tooling }}"
          : "if: matrix.target == 'x86_64-apple-darwin'"
      );
      expect(workflow).toContain('if [ ! -x scripts/run-native-verification.sh ]; then');
      expect(workflow).toContain(
        "::error::candidate predates #547 Rosetta tooling; the owner reruns with native-release rerun=<tag>'\n            exit 1"
      );
    }
  });
});
