import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { FixtureRegistration, registerFixture, setup } from '../e2e/harness/lifecycle.js';
import { killAndWait } from '../e2e/harness/cleanup.js';

const gone = Object.assign(new Error('group gone'), { code: 'ESRCH' });
const unknown = Object.assign(new Error('inspection denied'), { code: 'EPERM' });

afterEach(() => {
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
