import { BrowserAction } from '@tmt/browser-ui/react';
import { ColabHeader } from './colab-header.js';
import { NoticeCard } from './notice-card.js';
import { ReaderApp } from './reader-app.js';
import type { HostedEntryState } from './hosted-entry.js';
import { text } from './strings.js';
import './hosted-app.css';

export { EditRequestNotice as HostedRequestNotice } from './reader-source.js';
export type { EditRequestOutcome as HostedRequestOutcome } from './reader-source.js';

const signInCopy = {
  viewer:
    'Sign in to read the shared page. Signing in gives read access only; you cannot edit the page.',
  commenter:
    'Sign in to read the shared page. Signing in gives read access only; you cannot edit the page.',
  editor:
    'Sign in to read this page and send edit requests. The owner applies edits while Colab is running.',
};

/** Fixed copy only: raw provider/crypto diagnostics, keys and tokens never reach presentation. */
export function hostedFailureCopy(
  failure: unknown,
  presentation: { nowMs?: number; timeZone?: string } = {},
): {
  title: string;
  body: string;
  reset?: string;
  state?: 'inactive';
  stateLabel?: string;
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
          ? new Date(value.resetAtMs)
          : undefined;
      let resetLine: string | undefined;
      if (reset) {
        const zone = { timeZone: presentation.timeZone };
        const day = new Intl.DateTimeFormat('en-US', {
          ...zone,
          year: 'numeric',
          month: 'numeric',
          day: 'numeric',
        });
        const time = new Intl.DateTimeFormat('en-US', {
          ...zone,
          hour: '2-digit',
          minute: '2-digit',
          hourCycle: 'h23',
        }).format(reset);
        const sameDay = day.format(reset) === day.format(presentation.nowMs ?? Date.now());
        const date = new Intl.DateTimeFormat('en-US', {
          ...zone,
          month: 'short',
          day: 'numeric',
        }).format(reset);
        resetLine = sameDay
          ? `The allowance resets at ${time}.`
          : `The allowance resets on ${date} at ${time}.`;
      }
      return {
        title: 'Daily reading allowance reached',
        body: 'Reading has stopped in this browser for today. Open the link again after the allowance resets.',
        state: 'inactive',
        stateLabel: 'Limit reached',
        ...(resetLine ? { reset: resetLine } : {}),
      };
    }
    case 'REMOTE_RATE_LIMITED':
      return {
        title: 'Too many requests',
        body: 'Reading is paused for a moment. Try again in a few minutes.',
        state: 'inactive',
        stateLabel: 'Paused',
      };
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
          state={{ kind: 'ready', view: state.view, pageId: state.pageId }}
          canEdit={state.role === 'editor'}
          edit={state.role === 'editor' ? state.edit : undefined}
          accessNote={
            state.role === 'editor' ? 'The owner applies edits while Colab is running.' : undefined
          }
          notice={
            state.budget.state === 'warn' ? (
              <p className="hosted-reading-warning" role="status">
                This browser is close to its daily reading allowance. Reads in other tabs count too.
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
              ? (failure?.state ?? 'blocked')
              : state.phase === 'paused'
                ? 'inactive'
                : state.phase === 'sign-in' || state.phase === 'link-needed'
                  ? 'waiting'
                  : 'opening'
          }
          stateLabel={
            failure?.stateLabel ??
            (state.phase === 'sign-in'
              ? 'Sign-in needed'
              : state.phase === 'link-needed'
                ? 'Share link needed'
                : undefined)
          }
          eyebrow={state.phase === 'sign-in' && state.role === 'editor' ? 'Can edit' : 'Read-only'}
          title={title}
          testId="hosted-entry"
          actions={
            state.phase === 'sign-in' ? (
              <BrowserAction
                type="button"
                variant="text"
                label="Sign in"
                onActivate={(event) => {
                  if (event.isTrusted) signIn();
                }}
              />
            ) : undefined
          }
        >
          {state.phase === 'sign-in' && <p>{signInCopy[state.role ?? 'viewer']}</p>}
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
