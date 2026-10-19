import { useState } from 'react';
import type { Root } from 'react-dom/client';
import { BrowserAction, BrowserCommand, BrowserHeader } from '@tmt/browser-ui/react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { digest } from '@tmt/colab-client';
import type { HostedAuth } from '../../../../tmt-remote/typescript/remote-client/src/firebase-auth.js';
import type { HostedRemoteSdk } from './hosted-bootstrap.js';
import { remoteSdk, responseBytes } from './registration.js';
import { hex } from './export.js';
import { text } from './strings.js';

export interface ConnectGuideState {
  phase: 'checking' | 'sign-in' | 'signing-in' | 'signed-in' | 'failed';
  /** Available for copying only after Remote has validated the original public link. */
  remoteLink?: string;
}
interface Options {
  state(state: ConnectGuideState): void;
  fetch?: typeof fetch;
  load?(): Promise<Pick<HostedRemoteSdk, 'firebaseRemote'>>;
}

/** Owns one Auth reference, not page admission or a second Remote link parser. */
export function createConnectGuide(link: string, options: Options) {
  const lifetime = new AbortController();
  let closed = false;
  let auth: HostedAuth | undefined;
  let phase: ConnectGuideState['phase'] = 'checking';
  let pending: Promise<void> | undefined;
  let closing: Promise<void> | undefined;
  const publish = (next: ConnectGuideState) => {
    if (!closed) {
      phase = next.phase;
      options.state(next);
    }
  };
  const initialize = async () => {
    publish({ phase: 'checking' });
    try {
      const response = await (options.fetch ?? globalThis.fetch.bind(globalThis))(
        '/tmt/declarations/colab.json',
        {
          credentials: 'omit',
          redirect: 'error',
          cache: 'no-store',
          signal: AbortSignal.any([lifetime.signal, AbortSignal.timeout(10_000)]),
        },
      );
      const raw = await responseBytes(response, 65_536);
      if (!response.ok || closed) {
        if (!closed) publish({ phase: 'failed' });
        return;
      }
      const declaration = {
        bytes: new TextDecoder('utf-8', { fatal: true }).decode(raw),
        digest: hex(await digest(raw)),
      };
      const sdk = options.load
        ? await options.load()
        : ((await remoteSdk()) as unknown as Pick<HostedRemoteSdk, 'firebaseRemote'>);
      if (closed) return;
      // The same SDK check verifies the canonical carrier, declaration and same-origin config.
      const owner = await sdk.firebaseRemote.connect(link, declaration);
      if (closed) {
        await owner.close();
        return;
      }
      auth = owner;
      publish({ phase: 'sign-in', remoteLink: link });
    } catch {
      publish({ phase: 'failed' });
    }
  };
  const ready = initialize();
  return {
    ready,
    signIn(): Promise<void> {
      if (pending) return pending;
      if (closed || (phase !== 'sign-in' && phase !== 'failed')) return Promise.resolve();
      pending = (async () => {
        if (!auth) await initialize();
        if (closed || !auth) return;
        const owner = auth;
        publish({ phase: 'signing-in', remoteLink: link });
        try {
          await owner.signInAnonymous();
          publish({ phase: 'signed-in', remoteLink: link });
        } catch {
          publish({ phase: 'failed', remoteLink: link });
        }
      })().finally(() => {
        pending = undefined;
      });
      return pending;
    },
    close(): Promise<void> {
      if (!closing) {
        closed = true;
        lifetime.abort();
        closing = (async () => {
          await ready;
          try {
            await auth?.close();
          } finally {
            await pending;
          }
        })();
      }
      return closing;
    },
  };
}

/** Clipboard feedback says only what copying did, never that a machine step ran. */
function GuideCommand({ command, enabled }: { command: string; enabled: boolean }) {
  const [feedback, setFeedback] = useState('');
  return (
    <BrowserCommand
      text={command}
      action={
        <button
          type="button"
          className={ui.commandCopy}
          disabled={!enabled}
          onClick={async (event) => {
            if (!event.isTrusted) return;
            try {
              await navigator.clipboard.writeText(command);
              setFeedback(text.shellCommandCopied);
            } catch {
              setFeedback(text.shellCommandCopyFailed);
            }
          }}
        >
          Copy
        </button>
      }
      feedback={feedback}
    />
  );
}

export function ConnectGuide({ state, signIn }: { state: ConnectGuideState; signIn(): void }) {
  const label = {
    checking: 'Checking connection…',
    'sign-in': '',
    'signing-in': 'Signing in…',
    'signed-in': 'Signed in',
    failed: 'Could not sign in',
  }[state.phase];
  return (
    <>
      <BrowserHeader productLabel="Remote" title="Connect to this remote" />
      <main className="home">
        <p className="intro">
          Follow these steps to connect. Access to a shared page comes from its owner.
        </p>
        <section aria-labelledby="connect-browser">
          <h2 id="connect-browser">In this browser</h2>
          {label && (
            <p role={state.phase === 'failed' ? 'alert' : 'status'}>
              <span aria-hidden="true">
                {state.phase === 'failed' ? '✗' : state.phase === 'signed-in' ? '✓' : '◆'}
              </span>{' '}
              {label}
            </p>
          )}
          {(state.phase === 'sign-in' || state.phase === 'failed') && (
            <BrowserAction
              type="button"
              variant="text"
              label="Continue without an account"
              onActivate={(event) => {
                if (event.isTrusted) signIn();
              }}
            />
          )}
          <p>Signing in does not give access to every page.</p>
          <p>Open the page link its owner sent you. You join with the role that link gives.</p>
        </section>
        <section aria-labelledby="connect-machine">
          <h2 id="connect-machine">On this machine</h2>
          <ol>
            <li>
              <h3>Add the remote</h3>
              <p>Copy this command to remember the remote on this machine.</p>
              <GuideCommand
                command={
                  state.remoteLink
                    ? `tmt remote add team '${state.remoteLink}'`
                    : 'tmt remote add team <remote-link>'
                }
                enabled={state.remoteLink !== undefined}
              />
            </li>
            <li>
              <h3>Sign in</h3>
              <p>Copy this command to sign this machine in. No account is needed.</p>
              <GuideCommand
                command="tmt remote --remote team sync sign-in"
                enabled={state.remoteLink !== undefined}
              />
            </li>
          </ol>
        </section>
      </main>
    </>
  );
}

/** The entry erased the public fragment before routing it here instead of to the reader parser. */
export function startConnectGuide(root: Pick<Root, 'render'>, link: string) {
  const guide = createConnectGuide(link, {
    state: (state) =>
      root.render(<ConnectGuide state={state} signIn={() => void guide.signIn()} />),
  });
  const hide = () => void guide.close();
  const show = (event: PageTransitionEvent) => {
    if (event.persisted) location.reload();
  };
  const changed = () => location.reload();
  window.addEventListener('pagehide', hide, { once: true });
  window.addEventListener('pageshow', show);
  window.addEventListener('hashchange', changed);
  return async () => {
    window.removeEventListener('pagehide', hide);
    window.removeEventListener('pageshow', show);
    window.removeEventListener('hashchange', changed);
    await guide.close();
  };
}
