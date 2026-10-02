import { afterEach, describe, expect, it, vi } from 'vitest';
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
