import { describe, expect, it, vi } from 'vite-plus/test';
import {
  REOPEN_REASONS,
  ReopenFailedError,
  register,
  type RemoteSdk,
} from '../src/registration.js';

const mount = new URL('https://example.test/colab/');
// The SDK's class arrives at run time from /sdk/remote-v1.js, so it is recognised by name.
class SdkReopenError extends Error {
  constructor(readonly reason: string) {
    super(`Remote session admission: ${reason}`);
    this.name = 'ReopenSessionError';
  }
}
const sdkWith = (reopenSession: RemoteSdk['reopenSession']): RemoteSdk => ({
  reopenSession,
  certifyKey: vi.fn(),
  transportUrl: vi.fn(),
});

describe('register: re-admission of a Session that ended', () => {
  it('asks the SDK for a bounded series on the previous Session, under the caller signal', async () => {
    const previous = {};
    const signal = new AbortController().signal;
    const reopenSession = vi.fn(async () => {
      throw new SdkReopenError('transient');
    });
    await expect(
      register(mount, sdkWith(reopenSession), { previous, signal }),
    ).rejects.toBeInstanceOf(ReopenFailedError);
    expect(reopenSession).toHaveBeenCalledExactlyOnceWith(previous, { retry: 'bounded', signal });
  });

  it.each(REOPEN_REASONS)(
    'keeps the SDK reason %s as a typed error with the SDK error as cause',
    async (reason) => {
      const refusal = new SdkReopenError(reason);
      const failure = await register(
        mount,
        sdkWith(async () => {
          throw refusal;
        }),
        { previous: {} },
      ).catch((error: unknown) => error);
      expect(failure).toBeInstanceOf(ReopenFailedError);
      expect(failure).toMatchObject({ reason, cause: refusal });
    },
  );

  it('leaves any other failure as it was, so a network error stays a network error', async () => {
    const network = new TypeError('Failed to fetch');
    await expect(
      register(
        mount,
        sdkWith(async () => {
          throw network;
        }),
        { previous: {} },
      ),
    ).rejects.toBe(network);
    const unknown = new SdkReopenError('not-a-reason');
    await expect(
      register(
        mount,
        sdkWith(async () => {
          throw unknown;
        }),
        { previous: {} },
      ),
    ).rejects.toBe(unknown);
  });

  it('keeps the first boot a one-shot reopen with no options', async () => {
    const reopenSession = vi.fn(async () => {
      throw new SdkReopenError('unpaired');
    });
    await expect(register(mount, sdkWith(reopenSession))).rejects.toBeInstanceOf(SdkReopenError);
    expect(reopenSession).toHaveBeenCalledExactlyOnceWith();
  });
});
