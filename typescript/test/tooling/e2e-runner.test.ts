import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { createSandbox, runCli } from '../support/cli-process.js';

describe('Docker wrapper executable forwarding', () => {
  it.each([0, 23])('runs and cleans only the admitted CI image with status %s', (status) => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-e2e-prepared-'));
    try {
      const log = path.join(root, 'docker.jsonl');
      const image = 'tmux-team-e2e:ci-123-docker-e2e-shard-1';
      writeExecutable(
        path.join(root, 'docker'),
        `#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.TMT_RUNNER_LOG, JSON.stringify(args) + '\\n');
process.exitCode = args[0] === 'run' ? ${status} : 0;
`,
        0o755
      );
      const result = spawnSync(
        process.execPath,
        [fileURLToPath(new URL('../../scripts/run-e2e.mjs', import.meta.url))],
        {
          encoding: 'utf8',
          timeout: 10_000,
          env: {
            ...process.env,
            PATH: `${root}${path.delimiter}${process.env.PATH ?? ''}`,
            TMT_RUNNER_LOG: log,
            CI: 'true',
            GITHUB_ACTIONS: 'true',
            TMT_E2E_PREPARED_IMAGE: image,
            TMT_E2E_FILES: 'ops.e2e.test.ts',
            TMT_E2E_ADAPTER_TESTS: '0',
            CARGO_BUILD_JOBS: '',
            TMT_TEST_CLI: '',
            TMT_TEST_PEER_CLI: '',
          },
        }
      );
      expect(result.error).toBeUndefined();
      expect(result.status).toBe(status);
      expect(result.stderr).toBe('');
      const calls = fs
        .readFileSync(log, 'utf8')
        .trim()
        .split('\n')
        .map((line) => JSON.parse(line));
      expect(calls).toEqual([
        [
          'run',
          '--rm',
          '--init',
          '--network',
          'none',
          '--env',
          'TMT_TEST_CLI=',
          '--env',
          'TMT_TEST_PEER_CLI=',
          '--env',
          'TMT_E2E_FILES=ops.e2e.test.ts',
          '--env',
          'TMT_E2E_ADAPTER_TESTS=0',
          image,
        ],
        ['image', 'rm', '--force', image],
      ]);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  it.each([
    ['tmux-team-e2e:ci-123-docker-e2e-shard-1', '', 'true'],
    ['tmux-team-e2e:ci-123-docker-e2e-shard-1', 'true', ''],
    ['other:latest', 'true', 'true'],
    ['tmux-team-e2e:ci-123-docker-e2e-shard-3', 'true', 'true'],
    ['tmux-team-e2e:ci-0-docker-e2e-shard-1', 'true', 'true'],
  ])('refuses an unadmitted prepared image %s (%s/%s)', (image, ci, actions) => {
    const result = spawnSync(
      process.execPath,
      [fileURLToPath(new URL('../../scripts/run-e2e.mjs', import.meta.url))],
      {
        encoding: 'utf8',
        timeout: 10_000,
        env: {
          ...process.env,
          PATH: '',
          CI: ci,
          GITHUB_ACTIONS: actions,
          TMT_E2E_PREPARED_IMAGE: image,
          TMT_E2E_FILES: '',
          TMT_E2E_ADAPTER_TESTS: '',
          CARGO_BUILD_JOBS: '',
        },
      }
    );
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(2);
    expect(result.stdout).toBe('');
    expect(result.stderr).toBe(
      'TMT_E2E_PREPARED_IMAGE requires the explicit GitHub CI shard image.\n'
    );
  });

  it('preserves a failed image build without running the container', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-e2e-build-failure-'));
    try {
      const log = path.join(root, 'docker.jsonl');
      writeExecutable(
        path.join(root, 'docker'),
        `#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.TMT_RUNNER_LOG, JSON.stringify(args) + '\\n');
process.exitCode = args[0] === 'build' ? 23 : 0;
`,
        0o755
      );
      const result = spawnSync(
        process.execPath,
        [fileURLToPath(new URL('../../scripts/run-e2e.mjs', import.meta.url))],
        {
          encoding: 'utf8',
          timeout: 10_000,
          env: {
            ...process.env,
            PATH: `${root}${path.delimiter}${process.env.PATH ?? ''}`,
            TMT_RUNNER_LOG: log,
            TMT_E2E_FILES: '',
            TMT_E2E_ADAPTER_TESTS: '',
            CARGO_BUILD_JOBS: '',
          },
        }
      );
      expect(result.error).toBeUndefined();
      expect(result.status).toBe(23);
      expect(result.signal).toBeNull();
      expect(result.stdout).toBe('');
      expect(result.stderr).toBe('');
      const calls = fs
        .readFileSync(log, 'utf8')
        .trim()
        .split('\n')
        .map((line) => JSON.parse(line) as string[]);
      expect(calls).toHaveLength(2);
      expect(calls[0].slice(0, 2)).toEqual(['build', '--tag']);
      expect(calls[1]).toEqual(['image', 'rm', '--force', calls[0][2]]);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  it.each(['unset', 'selected', 'failed container'])(
    'preserves argv, isolation and image cleanup with %s settings',
    async (mode) => {
      const sandbox = createSandbox({
        TMT_TEST_CLI: JSON.stringify({
          executable: process.execPath,
          args: [fileURLToPath(new URL('../../scripts/run-e2e.mjs', import.meta.url))],
        }),
      });
      try {
        const bin = path.join(sandbox.root, 'fake docker');
        fs.mkdirSync(bin);
        const log = path.join(sandbox.root, 'docker.jsonl');
        writeExecutable(
          path.join(bin, 'docker'),
          `#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.TMT_RUNNER_LOG, JSON.stringify(args) + '\\n');
process.exitCode = args[0] === 'run' ? Number(process.env.TMT_RUNNER_STATUS) : 0;
`,
          0o755
        );
        sandbox.env.PATH = `${bin}${path.delimiter}${process.env.PATH ?? ''}`;
        sandbox.env.TMT_RUNNER_LOG = log;
        const status = mode === 'failed container' ? 23 : 0;
        sandbox.env.TMT_RUNNER_STATUS = String(status);
        delete sandbox.env.TMT_TEST_CLI;
        delete sandbox.env.TMT_TEST_PEER_CLI;
        const primary = JSON.stringify({
          executable: "/container/CLI's path",
          args: ['prefix with spaces', '$HOME; quoted'],
        });
        const peer = JSON.stringify({ executable: '/container/peer', args: [] });
        if (mode !== 'unset') {
          sandbox.env.TMT_TEST_CLI = primary;
          sandbox.env.TMT_TEST_PEER_CLI = peer;
        }
        expect(await runCli(sandbox, [])).toEqual({ status, signal: null, stdout: '', stderr: '' });
        const calls = fs
          .readFileSync(log, 'utf8')
          .trim()
          .split('\n')
          .map((line) => JSON.parse(line) as string[]);
        expect(calls).toHaveLength(3);
        expect(calls[0].slice(0, 2)).toEqual(['build', '--tag']);
        const image = calls[0][2];
        expect(calls[1]).toEqual([
          'run',
          '--rm',
          '--init',
          '--network',
          'none',
          ...(mode === 'unset'
            ? []
            : ['--env', `TMT_TEST_CLI=${primary}`, '--env', `TMT_TEST_PEER_CLI=${peer}`]),
          image,
        ]);
        expect(calls[2]).toEqual(['image', 'rm', '--force', image]);
      } finally {
        fs.rmSync(sandbox.root, { recursive: true, force: true });
      }
    }
  );

  /** Runs the wrapper against a fake `docker` that logs its argv, with TMT_E2E_FILES set. */
  async function runWrapper(files: string, adapterTests = '', cargoJobs = '') {
    const sandbox = createSandbox({
      TMT_TEST_CLI: JSON.stringify({
        executable: process.execPath,
        args: [fileURLToPath(new URL('../../scripts/run-e2e.mjs', import.meta.url))],
      }),
    });
    try {
      const bin = path.join(sandbox.root, 'fake docker');
      fs.mkdirSync(bin);
      const log = path.join(sandbox.root, 'docker.jsonl');
      writeExecutable(
        path.join(bin, 'docker'),
        `#!/usr/bin/env node
require('node:fs').appendFileSync(process.env.TMT_RUNNER_LOG, JSON.stringify(process.argv.slice(2)) + '\\n');
`,
        0o755
      );
      sandbox.env.PATH = `${bin}${path.delimiter}${process.env.PATH ?? ''}`;
      sandbox.env.TMT_RUNNER_LOG = log;
      delete sandbox.env.TMT_TEST_CLI;
      delete sandbox.env.TMT_TEST_PEER_CLI;
      sandbox.env.TMT_E2E_FILES = files;
      sandbox.env.TMT_E2E_ADAPTER_TESTS = adapterTests;
      sandbox.env.CARGO_BUILD_JOBS = cargoJobs;
      const result = await runCli(sandbox, []);
      const calls = fs.existsSync(log)
        ? fs
            .readFileSync(log, 'utf8')
            .trim()
            .split('\n')
            .map((line) => JSON.parse(line) as string[])
        : [];
      return { result, calls };
    } finally {
      fs.rmSync(sandbox.root, { recursive: true, force: true });
    }
  }

  it.each([
    ['ops.e2e.test.ts', ['--env', 'TMT_E2E_FILES=ops.e2e.test.ts']],
    ['a.e2e.test.ts b.e2e.test.ts', ['--env', 'TMT_E2E_FILES=a.e2e.test.ts b.e2e.test.ts']],
    ['', []],
  ])(
    'passes the file list %j to the container as one environment value',
    async (files, expected) => {
      const { result, calls } = await runWrapper(files);
      expect(result).toEqual({ status: 0, signal: null, stdout: '', stderr: '' });
      expect(calls[1].slice(0, 5)).toEqual(['run', '--rm', '--init', '--network', 'none']);
      expect(calls[1].slice(5, -1)).toEqual(expected);
    }
  );

  it.each(['../ops.e2e.test.ts', 'a;b', 'a  b', ' a', '$HOME', 'a\nb', 'ops*'])(
    'rejects the unsafe file list %j before building anything',
    async (files) => {
      const { result, calls } = await runWrapper(files);
      expect(result.status).toBe(2);
      expect(result.stderr).toContain('TMT_E2E_FILES must be a space-separated list');
      expect(calls).toEqual([]);
    }
  );

  it.each([
    ['0', ['--env', 'TMT_E2E_ADAPTER_TESTS=0']],
    ['1', ['--env', 'TMT_E2E_ADAPTER_TESTS=1']],
    ['', []],
  ])(
    'passes the adapter-test flag %j to the container as one environment value',
    async (flag, expected) => {
      const { result, calls } = await runWrapper('', flag);
      expect(result).toEqual({ status: 0, signal: null, stdout: '', stderr: '' });
      expect(calls[1].slice(5, -1)).toEqual(expected);
    }
  );

  it('forwards the file list and the adapter flag together', async () => {
    const { calls } = await runWrapper('a.e2e.test.ts b.e2e.test.ts', '0');
    expect(calls[1].slice(5, -1)).toEqual([
      '--env',
      'TMT_E2E_FILES=a.e2e.test.ts b.e2e.test.ts',
      '--env',
      'TMT_E2E_ADAPTER_TESTS=0',
    ]);
  });

  it.each(['2', 'yes', '00', '0 1', ' 0', 'true', '$HOME'])(
    'rejects the adapter-test flag %j before building anything',
    async (flag) => {
      const { result, calls } = await runWrapper('', flag);
      expect(result.status).toBe(2);
      expect(result.stderr).toContain('TMT_E2E_ADAPTER_TESTS must be 0 or 1');
      expect(calls).toEqual([]);
    }
  );

  it.each([
    ['2', ['--build-arg', 'CARGO_BUILD_JOBS=2']],
    ['default', ['--build-arg', 'CARGO_BUILD_JOBS=default']],
    ['', []],
  ])(
    'passes the cargo job limit %j to the image build as one build argument',
    async (jobs, expected) => {
      const { result, calls } = await runWrapper('', '', jobs);
      expect(result).toEqual({ status: 0, signal: null, stdout: '', stderr: '' });
      expect(calls[0].slice(0, 3)).toEqual(['build', '--tag', calls[1].at(-1)]);
      expect(calls[0].slice(3, -3)).toEqual(expected);
    }
  );

  it.each(['0', '-1', '02', '2 ', 'all', '$HOME', '2;id'])(
    'rejects the cargo job limit %j before building anything',
    async (jobs) => {
      const { result, calls } = await runWrapper('', '', jobs);
      expect(result.status).toBe(2);
      expect(result.stderr).toContain('CARGO_BUILD_JOBS must be a positive integer or default');
      expect(calls).toEqual([]);
    }
  );
});
