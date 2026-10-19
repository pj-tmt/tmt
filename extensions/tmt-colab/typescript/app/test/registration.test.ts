import { describe, expect, it, vi } from 'vite-plus/test';
import {
  REOPEN_REASONS,
  ReopenFailedError,
  register,
  jsonResponse,
  type RemoteSdk,
} from '../src/registration.js';
import { SessionLimitError } from '../src/session-limit-error.js';

const mount = new URL('https://example.test/colab/');
// The SDK's class arrives at run time from /sdk/remote-v1.js, so it is recognised by name.
class SdkReopenError extends Error {
  constructor(readonly reason: string) {
    super(`Remote session admission: ${reason}`);
    this.name = 'ReopenSessionError';
  }
}
// The SDK's refusal carries the verified limit; it is recognised by name too.
class SdkRefusal extends Error {
  constructor(
    readonly code: string,
    readonly limit?: unknown,
    readonly settingsUrl?: unknown,
  ) {
    super(`Remote operation refused: ${code}.`);
    this.name = 'RefusalError';
  }
}
const sdkWith = (reopenSession: RemoteSdk['reopenSession']): RemoteSdk => ({
  reopenSession,
  certifyKey: vi.fn(),
  transportUrl: vi.fn(),
});

describe('HTTP registration admission refusals', () => {
  it('keeps the HTTP status, refusal code and bounded body', async () => {
    const body = '{"code":"DENIED"}';
    const refusal = await jsonResponse(new Response(body, { status: 403 }), 8192).catch(
      (error: unknown) => error,
    );
    expect(refusal).toMatchObject({ status: 403, code: 'DENIED', body });
  });

  it.each(['', 'not a JSON refusal'])('retains an unclassified refusal body %s', async (body) => {
    const refusal = await jsonResponse(new Response(body || null, { status: 403 }), 8192).catch(
      (error: unknown) => error,
    );
    expect(refusal).toMatchObject({ status: 403, code: undefined, body });
  });

  it('keeps the response byte bound for a refusal too', async () => {
    await expect(jsonResponse(new Response('x'.repeat(9), { status: 403 }), 8)).rejects.toThrow(
      'Colab response exceeds limit',
    );
  });
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

describe('register: a device at its Remote session limit (#2625)', () => {
  const settingsUrl = 'https://127.0.0.1:9000/settings';
  const refusal = () => new SdkRefusal('REMOTE_SESSION_LIMIT', 2, settingsUrl);
  const failure = (call: () => Promise<unknown>) => call().catch((error: unknown) => error);

  it("turns the boot open's raw refusal into the typed limit with its settings link", async () => {
    const error = await failure(() =>
      register(
        mount,
        sdkWith(async () => {
          throw refusal();
        }),
      ),
    );
    expect(error).toBeInstanceOf(SessionLimitError);
    expect(error).toMatchObject({ limit: 2, settingsUrl });
  });

  it("turns a bounded re-admission's capacity into the same typed limit", async () => {
    const reopen = new SdkReopenError('capacity');
    reopen.cause = refusal();
    const error = await failure(() =>
      register(
        mount,
        sdkWith(async () => {
          throw reopen;
        }),
        { previous: {} },
      ),
    );
    expect(error).toBeInstanceOf(SessionLimitError);
    expect(error).toMatchObject({ limit: 2, settingsUrl });
  });

  it.each([0, -1, 1.5, Number.NaN, '2', undefined, null])(
    'does not trust a refusal whose limit is %s',
    async (limit) => {
      const bad = new SdkRefusal('REMOTE_SESSION_LIMIT', limit, settingsUrl);
      const error = await failure(() =>
        register(
          mount,
          sdkWith(async () => {
            throw bad;
          }),
        ),
      );
      expect(error).toBe(bad);
    },
  );

  it('keeps only a web URL as the settings link', async () => {
    for (const [given, kept] of [
      ['http://localhost:9000/settings', 'http://localhost:9000/settings'],
      ['javascript:alert(1)', undefined],
      ['not a url', undefined],
      [42, undefined],
    ] as const) {
      const error = await failure(() =>
        register(
          mount,
          sdkWith(async () => {
            throw new SdkRefusal('REMOTE_SESSION_LIMIT', 3, given);
          }),
        ),
      );
      expect(error).toMatchObject({ limit: 3, settingsUrl: kept });
    }
  });

  it('leaves every other refusal and a capacity without its refusal alone', async () => {
    const other = new SdkRefusal('REMOTE_DEVICE_REVOKED', 2, settingsUrl);
    const unverified = new SdkReopenError('capacity');
    for (const thrown of [other, unverified]) {
      const error = await failure(() =>
        register(
          mount,
          sdkWith(async () => {
            throw thrown;
          }),
          { previous: {} },
        ),
      );
      expect(error).not.toBeInstanceOf(SessionLimitError);
    }
    // The first, an unwrapped refusal on the readmission path, is returned unchanged.
  });
});
