import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { serveLockOwners, signalServeLockOwner } from '../acceptance/harness/serve-lock.js';

afterEach(() => vi.restoreAllMocks());

describe('acceptance serve-lock signalling', () => {
  const lock = '/private/world/colab/serve.lock';
  const recordedPid = 12345;

  it('does not signal a reused recorded PID that no longer owns the private lock', () => {
    const kill = vi.spyOn(process, 'kill').mockReturnValue(true);
    const currentOwner = recordedPid + 1;
    const probe = vi.fn(() => new Set([currentOwner]));
    expect(signalServeLockOwner(lock, recordedPid, 'SIGTERM', probe)).toBe(false);
    expect(probe).toHaveBeenCalledExactlyOnceWith(lock);
    expect(kill).not.toHaveBeenCalled();
  });

  it('signals only a recorded PID confirmed by the current private-lock probe', () => {
    const kill = vi.spyOn(process, 'kill').mockReturnValue(true);
    expect(signalServeLockOwner(lock, recordedPid, 'SIGTERM', () => new Set([recordedPid]))).toBe(
      true,
    );
    expect(kill).toHaveBeenCalledExactlyOnceWith(recordedPid, 'SIGTERM');
  });

  it('does not wait for a lock holder that exited before the signal', () => {
    vi.spyOn(process, 'kill').mockImplementation(() => {
      throw Object.assign(new Error('exited'), { code: 'ESRCH' });
    });
    expect(signalServeLockOwner(lock, recordedPid, 'SIGTERM', () => new Set([recordedPid]))).toBe(
      false,
    );
  });

  it('a missing private lock attests no owner', () => {
    expect(serveLockOwners('/nonexistent-colab-test-world/serve.lock')).toEqual(new Set());
  });
});
