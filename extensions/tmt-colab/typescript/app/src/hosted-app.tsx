import { BrowserAction } from '@tmt/browser-ui/react';
import { ColabHeader } from './colab-header.js';
import { NoticeCard } from './notice-card.js';
import { ReaderApp } from './reader-app.js';
import type { HostedEntryState } from './hosted-entry.js';
import { text } from './strings.js';
import './hosted-app.css';

/** Fixed copy only: raw provider/crypto diagnostics, keys and tokens never reach presentation. */
export function hostedFailureCopy(failure: unknown): {
  title: string;
  body: string;
  reset?: string;
} {
  const value =
    typeof failure === 'object' && failure !== null ? (failure as Record<string, unknown>) : {};
  switch (value.code) {
    case 'COLAB_HOSTED_RESHARE_REQUIRED':
      return {
        title: 'Ask for a new share link',
        body: 'This page changed its sharing since this link was made. Ask the owner to share it again.',
      };
    case 'COLAB_HOSTED_OLDER_LINK':
      return { title: text.readerOlderTitle, body: text.readerOlder };
    case 'COLAB_HOSTED_NOT_IN_CLOUD':
      return {
        title: 'Page not in the cloud',
        body: 'This page is not currently available in the cloud. Ask the owner to open Colab and check sync.',
      };
    case 'REMOTE_BUDGET_EXHAUSTED': {
      const reset =
        typeof value.resetAtMs === 'number' &&
        Number.isSafeInteger(value.resetAtMs) &&
        value.resetAtMs > 0 &&
        value.resetAtMs <= 8_640_000_000_000_000
          ? new Date(value.resetAtMs).toISOString()
          : undefined;
      return {
        title: 'Daily reading allowance reached',
        body: 'Hosted reading has stopped in this browser. Open the link again after the daily allowance resets.',
        ...(reset ? { reset: `The allowance resets at ${reset}.` } : {}),
      };
    }
    case 'REMOTE_RATE_LIMITED':
      return { title: 'Reading stopped', body: 'Firestore is limiting requests. Try again later.' };
    case 'REMOTE_SCOPE_DENIED':
      return {
        title: 'Link cannot read this page',
        body: 'Ask the owner for a current share link.',
      };
    default:
      return {
        title: 'Page unavailable',
        body: 'This page could not be opened. Ask the owner to check the share link.',
      };
  }
}

/** Hosted B4 exposes the reader only, even when a Rules-checked link role is editor. */
export function HostedApp({ state, signIn }: { state: HostedEntryState; signIn(): void }) {
  if (state.phase === 'ready')
    return (
      <div className="hosted-reader">
        <ReaderApp
          state={{ kind: 'ready', view: state.view }}
          notice={
            state.budget.state === 'warn' ? (
              <p className="hosted-reading-warning" role="status">
                This browser is close to its daily hosted reading allowance. Reads in other tabs
                count too.
              </p>
            ) : undefined
          }
        />
      </div>
    );
  const failure = state.phase === 'failed' ? hostedFailureCopy(state.failure) : undefined;
  const title =
    failure?.title ??
    (state.phase === 'sign-in'
      ? 'Read a shared page'
      : state.phase === 'link-needed'
        ? 'Open a share link'
        : state.phase === 'recovering'
          ? 'Reconnecting'
          : state.phase === 'paused'
            ? 'Reading paused'
            : 'Opening Colab');
  return (
    <>
      <ColabHeader title={title} />
      <main>
        <NoticeCard
          state={
            state.phase === 'failed'
              ? 'blocked'
              : state.phase === 'paused'
                ? 'inactive'
                : state.phase === 'sign-in' || state.phase === 'link-needed'
                  ? 'waiting'
                  : 'opening'
          }
          stateLabel={
            state.phase === 'sign-in'
              ? 'Firestore sign-in'
              : state.phase === 'link-needed'
                ? 'Share link needed'
                : undefined
          }
          eyebrow="Read-only"
          title={title}
          testId="hosted-entry"
          actions={
            state.phase === 'sign-in' ? (
              <BrowserAction
                type="button"
                variant="text"
                label="Sign in to Firestore"
                onActivate={(event) => {
                  if (event.isTrusted) signIn();
                }}
              />
            ) : undefined
          }
        >
          {state.phase === 'sign-in' && (
            <p>
              Sign in to Firestore to read the shared page. This does not give you editing access.
            </p>
          )}
          {(state.phase === 'link-needed' || (state.phase === 'sign-in' && !state.hasLink)) && (
            <p>Open a share link from Colab to read a page here.</p>
          )}
          {state.phase === 'loading' && (
            <p>Checking the connection and verifying the shared page.</p>
          )}
          {state.phase === 'recovering' && (
            <p>Reading has paused while the connection is restored.</p>
          )}
          {state.phase === 'paused' && (
            <p>
              Reading is paused while this tab is hidden. Return to this tab to check for changes.
            </p>
          )}
          {failure && (
            <>
              <p>{failure.body}</p>
              {failure.reset && <p>{failure.reset}</p>}
            </>
          )}
        </NoticeCard>
      </main>
    </>
  );
}
