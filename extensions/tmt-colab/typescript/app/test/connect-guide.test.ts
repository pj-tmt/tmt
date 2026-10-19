import { createHash } from 'node:crypto';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { ConnectGuide, createConnectGuide, type ConnectGuideState } from '../src/connect-guide.js';
import type { HostedAuth } from '../../../../tmt-remote/typescript/remote-client/src/firebase-auth.js';

const link = 'https://demo-guide.web.app/pair#public-config';
const declaration = '{"extension":"fixture"}\n';
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => (resolve = done));
  return { promise, resolve };
}
function fixture() {
  const states: ConnectGuideState[] = [];
  const calls = { connect: 0, signIn: 0, close: 0, paths: [] as string[] };
  const auth: HostedAuth = {
    sealedSync: async () => {
      throw new Error('the guide must never open a page transport');
    },
    signInAnonymous: async () => {
      calls.signIn++;
      return 'browser-uid';
    },
    signOut: async () => {
      throw new Error('the guide must not sign out another owner');
    },
    close: async () => {
      calls.close++;
    },
  };
  const options: Parameters<typeof createConnectGuide>[1] = {
    state: (state: ConnectGuideState) => states.push(state),
    fetch: (async (input: string | URL | Request, init?: RequestInit) => {
      calls.paths.push(String(input));
      expect(init?.credentials).toBe('omit');
      expect(init?.redirect).toBe('error');
      return new Response(declaration);
    }) as typeof fetch,
    load: async () => ({
      firebaseRemote: {
        originLink: async () => {
          throw new Error('do not replace the incoming link with another one');
        },
        connect: async (input: string, value: { bytes: string; digest: string }) => {
          calls.connect++;
          expect(input).toBe(link);
          expect(value).toEqual({
            bytes: declaration,
            digest: createHash('sha256').update(declaration).digest('hex'),
          });
          return auth;
        },
      },
    }),
  };
  return { states, calls, auth, options };
}

it('passes the original public link and exact declaration through Remote before enabling copy', async () => {
  const f = fixture();
  const guide = createConnectGuide(link, f.options);
  expect(f.states).toEqual([{ phase: 'checking' }]);
  await guide.ready;
  expect(f.calls.paths).toEqual(['/tmt/declarations/colab.json']);
  expect(f.calls.connect).toBe(1);
  expect(f.calls.signIn).toBe(0);
  expect(f.states.at(-1)).toEqual({ phase: 'sign-in', remoteLink: link });
  await guide.close();
  await guide.close();
  expect(f.calls.close).toBe(1);
});

it('observes one explicit browser sign-in and coalesces a second press without claiming machine access', async () => {
  const f = fixture();
  const signed = deferred<string>();
  f.auth.signInAnonymous = () => {
    f.calls.signIn++;
    return signed.promise;
  };
  const guide = createConnectGuide(link, f.options);
  await guide.ready;
  const first = guide.signIn();
  const second = guide.signIn();
  expect(first).toBe(second);
  expect(f.states.at(-1)?.phase).toBe('signing-in');
  signed.resolve('browser-uid');
  await first;
  expect(f.calls.signIn).toBe(1);
  expect(f.states.at(-1)).toEqual({ phase: 'signed-in', remoteLink: link });
  await guide.close();
});

it('never exposes a rejected link or raw provider failure and never signs in after validation refuses', async () => {
  const f = fixture();
  const unsafe = "https://demo-guide.web.app/pair#'$(secret-marker)'";
  f.options.load = async () => ({
    firebaseRemote: {
      originLink: async () => unsafe,
      connect: async () => {
        throw { code: 'REMOTE_INPUT_INVALID', message: 'secret-provider-body' };
      },
    },
  });
  const guide = createConnectGuide(unsafe, f.options);
  await guide.ready;
  await guide.signIn();
  expect(f.states.at(-1)).toEqual({ phase: 'failed' });
  expect(f.calls.signIn).toBe(0);
  const html = markup(f.states.at(-1)!);
  expect(html).not.toMatch(/secret-marker|secret-provider-body/);
  expect(html).toContain('Could not sign in');
  await guide.close();
});

it.each([404, 'oversized'] as const)(
  'refuses %s declaration input before connect',
  async (failure) => {
    const f = fixture();
    f.options.fetch = (async () =>
      failure === 404
        ? new Response('private-provider-detail', { status: 404 })
        : new Response('x'.repeat(65_537))) as typeof fetch;
    const guide = createConnectGuide(link, f.options);
    await guide.ready;
    expect(f.calls.connect).toBe(0);
    expect(f.calls.signIn).toBe(0);
    expect(f.states.at(-1)).toEqual({ phase: 'failed' });
    await guide.close();
  },
);

it('closes an Auth owner that arrives after the page closes without publishing sign-in readiness', async () => {
  const f = fixture();
  const connected = deferred<HostedAuth>();
  const started = deferred<void>();
  f.options.load = async () => ({
    firebaseRemote: {
      originLink: async () => link,
      connect: () => {
        started.resolve();
        return connected.promise;
      },
    },
  });
  const guide = createConnectGuide(link, f.options);
  await started.promise;
  const closing = guide.close();
  connected.resolve(f.auth);
  await closing;
  expect(f.states).toEqual([{ phase: 'checking' }]);
  expect(f.calls.signIn).toBe(0);
  expect(f.calls.close).toBe(1);
});

it('suppresses a late identity after close and joins the in-flight sign-in', async () => {
  const f = fixture();
  const signed = deferred<string>();
  f.auth.signInAnonymous = () => signed.promise;
  f.auth.close = async () => {
    f.calls.close++;
    signed.resolve('late-browser-uid');
  };
  const guide = createConnectGuide(link, f.options);
  await guide.ready;
  const pending = guide.signIn();
  await guide.close();
  await pending;
  expect(f.states.at(-1)?.phase).toBe('signing-in');
  expect(f.calls.close).toBe(1);
});

it('retries an unconfirmed sign-in only after another explicit press', async () => {
  const f = fixture();
  f.auth.signInAnonymous = async () => {
    f.calls.signIn++;
    throw new Error('private provider text');
  };
  const guide = createConnectGuide(link, f.options);
  await guide.ready;
  await guide.signIn();
  await Promise.resolve();
  expect(f.calls.signIn).toBe(1);
  expect(f.states.at(-1)).toEqual({ phase: 'failed', remoteLink: link });
  await guide.signIn();
  expect(f.calls.signIn).toBe(2);
  await guide.close();
});

function markup(state: ConnectGuideState) {
  return renderToStaticMarkup(createElement(ConnectGuide, { state, signIn() {} }));
}
it.each([
  ['checking', 'Checking connection…'],
  ['sign-in', 'Continue without an account'],
  ['signing-in', 'Signing in…'],
  ['signed-in', 'Signed in'],
  ['failed', 'Could not sign in'],
] as const)('renders the %s browser state with no machine progress claim', (phase, copy) => {
  const html = markup({ phase, ...(phase === 'checking' ? {} : { remoteLink: link }) });
  expect(html).toContain('Connect to this remote');
  expect(html).toContain(copy);
  if (phase === 'failed') expect(html).toContain('Continue without an account');
  expect(html).not.toContain('--json');
  expect(html).not.toContain('Signing in does not give access to every page. Open a page link');
  expect(html).toContain('Signing in does not give access to every page.');
  expect(html).toContain(
    'Open the page link its owner sent you. You join with the role that link gives.',
  );
  expect(html.indexOf('Open the page link its owner')).toBeLessThan(
    html.indexOf('On this machine'),
  );
  expect(html).toContain('Copy this command to sign this machine in. No account is needed.');
  expect(html).not.toMatch(
    /Not checked|>Done<|>Ready<|>Joined<|owner must admit|tmt remote join|status --layers/,
  );
  expect(html).toContain('tmt remote --remote team sync sign-in');
});

it('rechecks a failed connection before an explicit retry signs in', async () => {
  const f = fixture();
  const sdk = await f.options.load!();
  const connect = sdk.firebaseRemote.connect;
  let attempts = 0;
  sdk.firebaseRemote.connect = async (input: string, value: { bytes: string; digest: string }) => {
    if (++attempts === 1) throw new Error('private bootstrap failure');
    return connect(input, value);
  };
  f.options.load = async () => sdk;
  const guide = createConnectGuide(link, f.options);
  await guide.ready;
  expect(f.states.at(-1)).toEqual({ phase: 'failed' });
  expect(attempts).toBe(1);
  expect(f.calls.signIn).toBe(0);
  await guide.signIn();
  expect(attempts).toBe(2);
  expect(f.calls.signIn).toBe(1);
  expect(f.states.at(-1)).toEqual({ phase: 'signed-in', remoteLink: link });
  await guide.close();
});

it('joins a retried connection that resolves after close without signing in', async () => {
  const f = fixture();
  f.options.load = async () => {
    throw new Error('private bootstrap failure');
  };
  const guide = createConnectGuide(link, f.options);
  await guide.ready;
  const started = deferred<void>();
  const connected = deferred<HostedAuth>();
  f.options.load = async () => ({
    firebaseRemote: {
      originLink: async () => link,
      connect: () => {
        started.resolve();
        return connected.promise;
      },
    },
  });
  const retry = guide.signIn();
  // This is a gate, not an elapsed-time guess: the retry must own a fresh connection.
  await Promise.race([
    started.promise,
    retry.then(() => {
      throw new Error('retry completed without attempting a connection');
    }),
  ]);
  const closing = guide.close();
  connected.resolve(f.auth);
  await closing;
  await retry;
  expect(f.calls.close).toBe(1);
  expect(f.calls.signIn).toBe(0);
  expect(f.states.at(-1)?.phase).not.toBe('signed-in');
});
