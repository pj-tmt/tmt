import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { spawn } from 'node:child_process';
import * as childProcess from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  FixtureRegistration,
  groupLeader,
  paneOwnedGroups,
  registerFixture,
  setup,
  stopOwnedGroups,
} from '../e2e/harness/lifecycle.js';
import { locateTmux } from '../support/private-tmux.js';
import { killAndWait } from '../e2e/harness/cleanup.js';

vi.mock('node:child_process', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:child_process')>();
  return {
    ...actual,
    execFileSync: vi.fn(actual.execFileSync),
    spawnSync: vi.fn(actual.spawnSync),
  };
});

const gone = Object.assign(new Error('group gone'), { code: 'ESRCH' });
const unknown = Object.assign(new Error('inspection denied'), { code: 'EPERM' });

afterEach(() => {
  vi.resetAllMocks();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe('bounded E2E process group cleanup', () => {
  it('waits for transient unknown inspection to become proven absent', async () => {
    vi.useFakeTimers();
    const errors: Error[] = [];
    const signal = vi.spyOn(process, 'kill').mockImplementation((_pid, value) => {
      if (value === 'SIGKILL') return true;
      throw unknown;
    });
    const result = killAndWait([95401], 'owned worker', (error) => errors.push(error));
    expect(errors).toEqual([]);
    signal.mockImplementation(() => {
      throw gone;
    });
    await vi.advanceTimersByTimeAsync(25);
    expect(await result).toEqual([]);
    expect(errors).toEqual([]);
    expect(signal).toHaveBeenCalledWith(-95401, 'SIGKILL');
  });

  it('reports unknown inspection and a surviving group at the bound', async () => {
    vi.useFakeTimers();
    const errors: Error[] = [];
    vi.spyOn(process, 'kill').mockImplementation((_pid, value) => {
      if (value === 'SIGKILL') return true;
      throw unknown;
    });
    const result = killAndWait([95402], 'owned worker', (error) => errors.push(error));
    await vi.advanceTimersByTimeAsync(1000);
    expect(await result).toEqual([95402]);
    expect(errors).toHaveLength(1);
    expect(errors[0].message).toContain('Could not inspect E2E owned worker process group 95402');
    expect(errors[0].cause).toBe(unknown);
  });

  it('preserves a failed signal even when later observation proves absence', async () => {
    const errors: Error[] = [];
    vi.spyOn(process, 'kill').mockImplementation((_pid, value) => {
      throw value === 'SIGKILL' ? unknown : gone;
    });
    expect(await killAndWait([95403], 'owned worker', (error) => errors.push(error))).toEqual([]);
    expect(errors).toHaveLength(1);
    expect(errors[0].message).toContain('Could not kill E2E owned worker process group 95403');
  });
});

describe('run-owned E2E fixture registry', () => {
  it('publishes allocations atomically and keeps a closing run from starting another', () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'e2e-registry-test-'));
    try {
      const owned = {
        root: '/private/fixture-root',
        socketRoot: '/private/fixture-socket-root',
        socketPath: '/private/fixture-socket-root/tmux-501/owned',
        tmuxPath: '/usr/bin/tmux',
        serverPid: 0,
        paneGroups: [],
        cliGroups: [],
      };
      const registration = new FixtureRegistration(directory, owned);
      expect(
        registration.allocate(
          () => 123,
          (pid) => (owned.serverPid = pid)
        )
      ).toBe(123);
      const files = fs.readdirSync(directory);
      expect(files).toHaveLength(1);
      expect(JSON.parse(fs.readFileSync(path.join(directory, files[0]), 'utf8'))).toEqual(owned);
      fs.writeFileSync(path.join(directory, '.closing'), '');
      const allocate = vi.fn(() => 456);
      expect(() => registration.allocate(allocate, () => {})).toThrow('E2E run is closing');
      expect(allocate).not.toHaveBeenCalled();
      registration.remove();
      expect(fs.readdirSync(directory)).toEqual(['.closing']);
    } finally {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });
});

describe('E2E suite survivor check', () => {
  it('passes a clean run and reports a retained fixture by its own roots', () => {
    expect(() => setup()()).not.toThrow();
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-e2e-unit-'));
    const socketParent =
      process.platform === 'darwin'
        ? fs.realpathSync('/private/tmp')
        : fs.realpathSync(os.tmpdir());
    const socketRoot = fs.mkdtempSync(path.join(socketParent, 'te2e-unit-'));
    try {
      const teardown = setup();
      registerFixture({
        root,
        socketRoot,
        socketPath: path.join(socketRoot, 'private.sock'),
        tmuxPath: '/usr/bin/tmux',
        serverPid: 0,
        paneGroups: [],
        cliGroups: [],
      });
      expect(teardown).toThrow(root);
      expect(fs.existsSync(root)).toBe(false);
      expect(fs.existsSync(socketRoot)).toBe(false);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
      fs.rmSync(socketRoot, { recursive: true, force: true });
    }
  });
});

describe('E2E group identity', () => {
  it('does not wait for or signal a reused process-group ID', async () => {
    const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
      detached: true,
      stdio: 'ignore',
    });
    const closed = new Promise<void>((resolve) => child.once('close', () => resolve()));
    try {
      await new Promise<void>((resolve, reject) => {
        child.once('spawn', resolve);
        child.once('error', reject);
      });
      const pid = child.pid!;
      const leader = groupLeader(pid);
      expect(leader).toBeDefined();
      const before = performance.now();
      stopOwnedGroups([{ pid, started: 'not the original start time' }]);
      expect(performance.now() - before).toBeLessThan(1_000);
      expect(groupLeader(pid)).toEqual(leader);
    } finally {
      if (child.pid && child.exitCode === null && child.signalCode === null) {
        process.kill(-child.pid, 'SIGKILL');
      }
      await closed;
    }
  });
});

describe('exited E2E group observations', () => {
  it.each(['-1', '4294967295'])('treats ps group %s as already gone', (pgid) => {
    vi.spyOn(childProcess, 'execFileSync').mockReturnValue(`${pgid} Wed Oct 28 04:09:33 2026`);
    const signal = vi.spyOn(process, 'kill').mockImplementation(() => {
      throw gone;
    });
    expect(groupLeader(95404)).toBeUndefined();
    expect(paneOwnedGroups(95404)).toEqual([]);
    expect(() =>
      stopOwnedGroups([{ pid: 95404, started: 'Wed Oct 28 04:09:33 2026' }])
    ).not.toThrow();
    expect(signal.mock.calls.every(([, value]) => value === 0)).toBe(true);
  });

  it('accepts exit while waiting for a verified group to stop', () => {
    vi.spyOn(childProcess, 'execFileSync')
      .mockReturnValueOnce('95407 known')
      .mockReturnValue('4294967295 known');
    let stopped = false;
    vi.spyOn(process, 'kill').mockImplementation((_pid, value) => {
      if (value === 'SIGTERM') {
        stopped = true;
        return true;
      }
      if (stopped) throw gone;
      return true;
    });
    expect(() => stopOwnedGroups([{ pid: 95407, started: 'known' }])).not.toThrow();
  });

  it.each(['-1', '4294967295'])('ignores an exited foreground group %s', (pgid) => {
    vi.spyOn(childProcess, 'execFileSync')
      .mockReturnValueOnce('95405 Wed Oct 28 04:09:33 2026')
      .mockReturnValueOnce(pgid);
    expect(paneOwnedGroups(95405)).toEqual([{ pid: 95405, started: 'Wed Oct 28 04:09:33 2026' }]);
  });

  it('keeps an unidentifiable surviving group failing', () => {
    vi.spyOn(childProcess, 'execFileSync').mockImplementation(() => {
      throw Object.assign(new Error('gone'), { status: 1 });
    });
    const signal = vi.spyOn(process, 'kill').mockReturnValue(true);
    expect(() => stopOwnedGroups([{ pid: 95406, started: 'known' }])).toThrow(
      'Cannot identify E2E group'
    );
    expect(signal.mock.calls.every(([, value]) => value === 0)).toBe(true);
  });
});

describe('stopped registered E2E server', () => {
  it('removes the stale socket at the first no-server observation without retrying absent resources', () => {
    const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-e2e-stale-')));
    const parent =
      process.platform === 'darwin'
        ? fs.realpathSync('/private/tmp')
        : fs.realpathSync(os.tmpdir());
    const socketRoot = fs.mkdtempSync(path.join(parent, 'te2e-stale-'));
    const socket = path.join(socketRoot, 'stale.sock');
    const teardown = setup();
    try {
      const tmuxPath = locateTmux();
      childProcess.execFileSync(process.execPath, [
        '-e',
        "require('node:net').createServer().listen(process.argv[1], () => process.exit(0))",
        socket,
      ]);
      registerFixture({
        root,
        socketRoot,
        socketPath: socket,
        tmuxPath,
        serverPid: 0,
        paneGroups: [],
        cliGroups: [],
      });
      const stderr = `no server running on ${socket}`;
      const query = vi.spyOn(childProcess, 'spawnSync').mockReturnValue({
        pid: 0,
        output: [null, '', stderr],
        stdout: '',
        stderr,
        status: 1,
        signal: null,
      });
      query.mockClear();
      // Registered fixtures still trigger the suite-end survivor verdict; cleanup itself succeeds.
      expect(teardown).toThrow(`E2E fixture survived suite end: ${root}`);
      expect(query).toHaveBeenCalledTimes(1);
      expect(fs.existsSync(root)).toBe(false);
      expect(fs.existsSync(socketRoot)).toBe(false);
    } finally {
      vi.restoreAllMocks();
      fs.rmSync(root, { recursive: true, force: true });
      fs.rmSync(socketRoot, { recursive: true, force: true });
      teardown();
    }
  });
});
