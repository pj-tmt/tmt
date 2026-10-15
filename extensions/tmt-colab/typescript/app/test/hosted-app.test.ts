import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { HostedApp, hostedFailureCopy } from '../src/hosted-app.js';
import type { HostedEntryState } from '../src/hosted-entry.js';

const markup = (state: HostedEntryState) =>
  renderToStaticMarkup(createElement(HostedApp, { state, signIn() {} }));
it('a fresh hosted load has real Firestore sign-in and a share-link next step', () => {
  const html = markup({ phase: 'sign-in', hasLink: false });
  expect(html).toMatch(/<button\b[^>]*>.*?Sign in to Firestore.*?<\/button>/);
  expect(html).toContain('share link');
  expect(html).not.toContain('sample pages');
  expect(html).not.toContain('anonymous');
});
it('only a verified ready view reaches the read-only reader and no writer controls appear', () => {
  const html = markup({
    phase: 'ready',
    view: { source: '<h1>Verified</h1>', title: 'Page' },
    budget: { reads: 3501, writes: 1, day: 1, state: 'warn' },
  });
  expect(html).toContain('Read-only');
  expect(html).toContain('daily hosted reading allowance');
  expect(html).not.toMatch(/>Save</);
  expect(html).not.toMatch(/>Ask</);
});
it.each(['loading', 'recovering', 'paused', 'link-needed'] as const)(
  'the %s state does not label retained content Live',
  (phase) => {
    const html = markup({ phase });
    expect(html).not.toContain('>Live<');
    expect(html).not.toContain('frame-host');
  },
);
it('later epochs use the fixed re-share copy without exposing provider diagnostics', () => {
  const failure = {
    code: 'COLAB_HOSTED_RESHARE_REQUIRED',
    message: '/private/secret-provider-detail',
  };
  expect(hostedFailureCopy(failure).body).toBe(
    'This page changed its sharing since this link was made. Ask the owner to share it again.',
  );
  expect(markup({ phase: 'failed', failure })).not.toContain(failure.message);
});
it('budget exhaustion shows the supplied reset and leaves no automatic retry action', () => {
  const failure = {
    code: 'REMOTE_BUDGET_EXHAUSTED',
    resetAtMs: Date.parse('2026-11-02T08:00:00Z'),
    retryAfterMs: 1000,
  };
  const html = markup({ phase: 'failed', failure });
  expect(html).toContain('2026-11-02T08:00:00.000Z');
  expect(html).not.toContain('<button');
});
it('an expired cloud copy and a Rules refusal have distinct bounded explanations', () => {
  expect(hostedFailureCopy({ code: 'COLAB_HOSTED_NOT_IN_CLOUD' }).title).toBe(
    'Page not in the cloud',
  );
  expect(hostedFailureCopy({ code: 'REMOTE_SCOPE_DENIED' }).body).toContain('current share link');
  expect(hostedFailureCopy({ code: 'REMOTE_RATE_LIMITED' }).body).toContain('Try again later');
});
