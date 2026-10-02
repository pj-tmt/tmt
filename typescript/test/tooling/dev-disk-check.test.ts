import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

const script = fileURLToPath(new URL('../../../scripts/dev-disk-check.sh', import.meta.url));

interface Fixture {
  root: string;
  env: NodeJS.ProcessEnv;
}

/** A task-owned directory for fake tools; a shell script needs no native build. */
async function withFixture(run: (fixture: Fixture) => void | Promise<void>) {
  const root = mkdtempSync(path.join(os.tmpdir(), 'dev-disk-check-'));
  try {
    await run({ root, env: {} });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

/** Puts fake `df` and (optionally) `docker` first on PATH; the fakes record their calls. */
function fakeTools(
  sandbox: Fixture,
  options: { freeGb: number; docker?: 'answers' | 'fails' | 'absent' }
) {
  const bin = path.join(sandbox.root, 'fake-bin');
  mkdirSync(bin);
  const calls = path.join(sandbox.root, 'calls.log');
  const tool = (name: string, body: string) => {
    writeExecutable(
      path.join(bin, name),
      `#!/bin/sh\necho "${name} $*" >> "${calls}"\n${body}\n`,
      0o755
    );
  };
  // POSIX `df -Pk` layout: the fourth column of the data row is available KiB.
  tool(
    'df',
    `printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\\n/dev/fake 999999999 1 ${options.freeGb * 1048576} 1%% /\\n'`
  );
  if (options.docker === 'answers') tool('docker', "echo 'TYPE TOTAL'; echo 'Images 3'");
  if (options.docker === 'fails') tool('docker', "echo 'daemon down' >&2; exit 1");
  sandbox.env.PATH = `${bin}${path.delimiter}/usr/bin:/bin`;
  return () => (existsSync(calls) ? readFileSync(calls, 'utf8').trim().split('\n') : []);
}

function check(sandbox: Fixture) {
  const result = spawnSync('/bin/sh', [script], {
    env: sandbox.env,
    encoding: 'utf8',
    timeout: 10_000,
  });
  return { status: result.status, stdout: result.stdout, stderr: result.stderr };
}

describe('scripts/dev-disk-check.sh', () => {
  it('warns below the threshold, names it, and reports Docker usage', async () => {
    await withFixture(async (sandbox) => {
      const calls = fakeTools(sandbox, { freeGb: 12, docker: 'answers' });
      const result = check(sandbox);
      expect(result.status).toBe(0);
      expect(result.stdout).toContain('Free disk space at');
      expect(result.stdout).toContain(': 12 GB');
      expect(result.stdout).toContain('WARNING: less than 30 GB is free');
      expect(result.stdout).toContain('tell the maintainer');
      expect(result.stdout).toContain('Images 3');
      // Read-only: only `df` and `docker system df` ran, once each.
      expect(calls().sort()).toEqual([expect.stringMatching(/^df -Pk /), 'docker system df']);
      expect(result.stderr).toBe('');
    });
  });

  it('is quiet about space above the threshold, and the boundary follows TMT_DISK_WARN_GB', async () => {
    await withFixture(async (sandbox) => {
      fakeTools(sandbox, { freeGb: 31, docker: 'answers' });
      const ample = check(sandbox);
      expect(ample.stdout).toContain(': 31 GB');
      expect(ample.stdout).not.toContain('WARNING');

      sandbox.env.TMT_DISK_WARN_GB = '31';
      expect(check(sandbox).stdout, 'exactly at the threshold').not.toContain('WARNING');
      sandbox.env.TMT_DISK_WARN_GB = '32';
      expect(check(sandbox).stdout).toContain('WARNING: less than 32 GB is free');
    });
  });

  it('still exits 0 when Docker is missing or does not answer', async () => {
    for (const docker of ['absent', 'fails'] as const) {
      await withFixture(async (sandbox) => {
        fakeTools(sandbox, { freeGb: 5, docker });
        const result = check(sandbox);
        expect(result.status, docker).toBe(0);
        expect(result.stdout).toContain('WARNING: less than 30 GB is free');
        if (docker === 'fails') expect(result.stdout).toContain('Docker did not answer');
        else expect(result.stdout).not.toContain('Docker did not answer');
      });
    }
  });

  it('rejects a threshold that is not a whole number before reading anything', async () => {
    await withFixture(async (sandbox) => {
      const calls = fakeTools(sandbox, { freeGb: 50, docker: 'answers' });
      sandbox.env.TMT_DISK_WARN_GB = '30GB';
      const result = check(sandbox);
      expect(result.status).toBe(2);
      expect(result.stderr).toContain('TMT_DISK_WARN_GB must be a whole number');
      expect(calls()).toEqual([]);
    });
  });
});
